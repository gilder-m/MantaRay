//! A pretend instrument for end-to-end tests, with faults on a script.
//!
//! Behind the `rig` feature, which only test builds turn on: nothing here may
//! reach a shipped binary, because a rig answers like an instrument and
//! counts like one, and its numbers are invented.
//!
//! The rig is the far end of the wire. It speaks the dialect
//! [`RemoteMcb`](crate::RemoteMcb) speaks (see `remote.rs`), keeps the state
//! a real MCB keeps (counting flag, clocks, preset registers, the histogram),
//! and stops itself on a held preset as the hardware does. The real control
//! code runs against it unchanged, on the direct line, over a courier, or
//! over TCP through the application's own connect path ([`Rig::serve`]).
//!
//! What makes it a rig rather than a simulator is the script of faults seen
//! on the bench or argued in review ([`Fault`]): a reply stream that slips by
//! one, a counting flag that lags after STOP, `ERR` replies, a status line
//! with a word missing, a front panel that changes things behind the
//! program's back, an instrument locked by someone else, a line that drops.
//!
//! Time is the rig's own: the clocks move only when the test calls
//! [`Rig::tick`] (or, with [`Rig::tick_per_status`], by a fixed step on
//! each status read). Nothing here reads the wall clock, so a test's outcome
//! never depends on how fast the machine running it is. The only waiting is
//! [`Rig::wait_until`], which blocks on a condition (a courier's fetch
//! landing) and fails after a generous deadline rather than hanging.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use crate::error::DeviceError;
use crate::transport::Transport;

/// The preset registers a real MCB holds. Zero (here `None`) means none set.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Registers {
    /// Real-time preset, seconds.
    pub real: Option<f64>,
    /// Live-time preset, seconds.
    pub live: Option<f64>,
    /// ROI peak-count preset.
    pub peak: Option<u64>,
    /// ROI integral preset.
    pub integral: Option<u64>,
}

/// A scripted misbehaviour. Each is armed with [`Rig::fault`] and spends
/// itself as it fires; several can be armed at once.
#[derive(Clone, Debug, PartialEq)]
pub enum Fault {
    /// The reply stream slips by one for the next `count` replies: each
    /// command is carried out, but answered with the reply that belonged to
    /// the command before it (ledger H-1, the bench 926's adapter). After
    /// the last slipped reply the stream is back in step; the reply that
    /// fell between is lost.
    Slip {
        /// How many replies come one behind.
        count: usize,
    },
    /// The next `count` commands whose verb is `verb` are answered with
    /// `reply`. With `execute` the command is carried out first (a command
    /// that worked but answered wrongly, as an idle 926 answering STOP with
    /// ERR); without it the instrument does nothing.
    Reply {
        /// The verb it applies to (`STOP`, `SHOW_STATUS`, ...).
        verb: String,
        /// What is answered instead.
        reply: String,
        /// How many times.
        count: usize,
        /// Whether the command still takes effect.
        execute: bool,
    },
    /// After the next accepted STOP, the counting flag keeps saying
    /// `ACTIVE=1` for this many status reads, though nothing counts.
    StopLag {
        /// Status reads that still say counting.
        reads: usize,
    },
    /// The next `count` status lines leave out the word `key=` (`ACTIVE`,
    /// `RT`, `LT`...).
    StatusWithout {
        /// The key left out.
        key: String,
        /// How many status lines.
        count: usize,
    },
    /// A self-stop on a time preset leaves that clock this many seconds
    /// short of the preset, as a tick counter read a moment early does.
    /// Stays armed.
    ClockShortfall {
        /// Seconds short.
        seconds: f64,
    },
    /// Every exchange from now on fails: the line has dropped.
    Disconnect,
}

/// One exchange as the rig saw it.
#[derive(Clone, Debug, PartialEq)]
pub struct Exchange {
    /// The command line received.
    pub command: String,
    /// The reply sent back.
    pub reply: String,
    /// Whether the instrument was counting when the command arrived.
    pub counting: bool,
}

/// The instrument's state. Reached through [`Rig`].
#[derive(Debug)]
struct Instrument {
    channels: Vec<u64>,
    counting: bool,
    real_time: f64,
    live_time: f64,
    dead_fraction: f64,
    counts_per_second: f64,
    /// Fractional counts carried between ticks, so a rate is exact over time.
    owed: f64,
    registers: Registers,
    high_voltage: f64,
    coarse_gain: f64,
    /// Someone else's lock: every changing command answers ERR.
    locked_by: Option<String>,
    faults: Vec<Fault>,
    /// Status reads that still say counting after a lagging STOP.
    lagging: usize,
    /// The reply the slipped stream owes the next command.
    behind: Option<String>,
    tick_per_status: f64,
    /// How long each exchange takes on the wire.
    latency: Duration,
    log: Vec<Exchange>,
    /// Commands that changed presets while counting: MAESTRO refuses them,
    /// so a program that sends one is wrong whatever the hardware does.
    violations: Vec<String>,
}

/// Shared handle to a rig: the test keeps one, every transport and served
/// connection holds another.
#[derive(Clone)]
pub struct Rig {
    shared: Arc<(Mutex<Instrument>, Condvar)>,
}

impl Rig {
    /// An idle instrument with this many channels and nothing held.
    pub fn new(channels: usize) -> Self {
        Self {
            shared: Arc::new((
                Mutex::new(Instrument {
                    channels: vec![0; channels],
                    counting: false,
                    real_time: 0.0,
                    live_time: 0.0,
                    dead_fraction: 0.0,
                    counts_per_second: 1000.0,
                    owed: 0.0,
                    registers: Registers::default(),
                    high_voltage: 0.0,
                    coarse_gain: 20.0,
                    locked_by: None,
                    faults: Vec::new(),
                    lagging: 0,
                    behind: None,
                    tick_per_status: 0.0,
                    latency: Duration::ZERO,
                    log: Vec::new(),
                    violations: Vec::new(),
                }),
                Condvar::new(),
            )),
        }
    }

    fn state(&self) -> MutexGuard<'_, Instrument> {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A transport on this rig, for [`RemoteMcb::connect`](crate::RemoteMcb::connect).
    pub fn transport(&self) -> Box<dyn Transport> {
        Box::new(RigTransport { rig: self.clone() })
    }

    /// Serves the rig over TCP on the loopback, for code that connects by
    /// address (the application's Network detector). Returns `host:port`.
    /// Each connection is served on its own thread, so a reconnect works.
    pub fn serve(&self) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("binding the rig's port");
        let address = listener
            .local_addr()
            .expect("the rig's address")
            .to_string();
        let rig = self.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let rig = rig.clone();
                std::thread::spawn(move || rig.serve_one(stream));
            }
        });
        address
    }

    fn serve_one(&self, stream: TcpStream) {
        let Ok(reading) = stream.try_clone() else {
            return;
        };
        let mut reader = BufReader::new(reading);
        let mut writer = stream;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            let command = line.trim();
            if command.is_empty() {
                continue;
            }
            match self.answer(command) {
                Ok(mut reply) => {
                    reply.push('\n');
                    if writer.write_all(reply.as_bytes()).is_err() {
                        return;
                    }
                }
                Err(_) => {
                    let _ = writer.shutdown(std::net::Shutdown::Both);
                    return;
                }
            }
        }
    }

    /// Arms a fault.
    pub fn fault(&self, fault: Fault) {
        self.state().faults.push(fault);
    }

    /// Answers `verb` with `ERR reason` the next `count` times, doing nothing.
    pub fn err(&self, verb: &str, reason: &str, count: usize) {
        self.fault(Fault::Reply {
            verb: verb.into(),
            reply: format!("ERR {reason}"),
            count,
            execute: false,
        });
    }

    /// Locks the instrument from elsewhere (another program, the front
    /// panel): every changing command answers `ERR locked by <owner>`.
    pub fn lock_elsewhere(&self, owner: &str) {
        self.state().locked_by = Some(owner.to_string());
    }

    /// Lifts a lock taken elsewhere.
    pub fn unlock_elsewhere(&self) {
        self.state().locked_by = None;
    }

    /// Advances the instrument's own clock by `seconds`: counts accumulate
    /// while counting, and a held preset stops it as the hardware would.
    pub fn tick(&self, seconds: f64) {
        let mut state = self.state();
        state.advance(seconds);
        drop(state);
        self.shared.1.notify_all();
    }

    /// Each status read from now on advances the clock this much first, so
    /// a loop that reads the status sees time pass without a wall clock.
    pub fn tick_per_status(&self, seconds: f64) {
        self.state().tick_per_status = seconds;
    }

    /// Makes every exchange take at least this long, as a slow adapter's
    /// round trip does. The rig's clocks still move only on [`Rig::tick`]:
    /// this is for code whose bound is (wrongly) in wall time, so that a
    /// test can show the bound does not depend on the wire's speed.
    pub fn set_latency(&self, latency: Duration) {
        self.state().latency = latency;
    }

    /// Sets the stored rate, counts per second of live time.
    pub fn set_rate(&self, counts_per_second: f64) {
        self.state().counts_per_second = counts_per_second;
    }

    /// Sets the dead fraction (live time = real time x (1 - this)).
    pub fn set_dead_fraction(&self, fraction: f64) {
        self.state().dead_fraction = fraction;
    }

    /// Someone at the front panel presses Start, behind the program's back.
    pub fn front_panel_start(&self) {
        self.state().start();
    }

    /// Someone at the front panel presses Stop.
    pub fn front_panel_stop(&self) {
        self.state().counting = false;
    }

    /// Someone at the front panel (or another program) sets the registers.
    pub fn front_panel_presets(&self, registers: Registers) {
        self.state().registers = registers;
    }

    /// Whether it is counting now.
    pub fn counting(&self) -> bool {
        self.state().counting
    }

    /// The preset registers as the instrument holds them.
    pub fn registers(&self) -> Registers {
        self.state().registers
    }

    /// The instrument's clocks, (real, live).
    pub fn clocks(&self) -> (f64, f64) {
        let state = self.state();
        (state.real_time, state.live_time)
    }

    /// The bias the instrument was last told, volts.
    pub fn high_voltage(&self) -> f64 {
        self.state().high_voltage
    }

    /// Every exchange so far.
    pub fn log(&self) -> Vec<Exchange> {
        self.state().log.clone()
    }

    /// The commands received so far, in order.
    pub fn sent(&self) -> Vec<String> {
        self.state()
            .log
            .iter()
            .map(|exchange| exchange.command.clone())
            .collect()
    }

    /// The commands received since the last call, and forgets them.
    pub fn take_sent(&self) -> Vec<String> {
        self.state()
            .log
            .drain(..)
            .map(|exchange| exchange.command)
            .collect()
    }

    /// Commands that changed presets while the instrument was counting.
    pub fn violations(&self) -> Vec<String> {
        self.state().violations.clone()
    }

    /// Blocks until `condition` holds of the rig's log (a courier's fetch has
    /// been served, say). Panics with `what` after ten seconds: the deadline
    /// only turns a hang into a failure, it never decides an outcome.
    pub fn wait_until(&self, what: &str, condition: impl Fn(&[Exchange]) -> bool) {
        let (lock, wake) = &*self.shared;
        let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !condition(&state.log) {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "the rig never saw: {what}");
            state = wake
                .wait_timeout(state, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// How many exchanges the rig has served so far.
    pub fn served(&self) -> usize {
        self.state().log.len()
    }

    /// Answers one command line, faults included. `Err` means the line dropped.
    fn answer(&self, command: &str) -> Result<String, DeviceError> {
        let latency = self.state().latency;
        if !latency.is_zero() {
            std::thread::sleep(latency);
        }
        let mut state = self.state();
        let outcome = state.answer(command);
        drop(state);
        self.shared.1.notify_all();
        outcome
    }
}

impl Instrument {
    fn start(&mut self) {
        // A held preset already satisfied: the 926 accepts START and does
        // not count (ortec-hardware.md). The rig does the same.
        if self.reached() {
            return;
        }
        self.counting = true;
    }

    fn reached(&self) -> bool {
        let registers = self.registers;
        registers
            .real
            .is_some_and(|preset| self.real_time >= preset)
            || registers
                .live
                .is_some_and(|preset| self.live_time >= preset)
            || registers
                .peak
                .is_some_and(|preset| self.channels.iter().copied().max().unwrap_or(0) >= preset)
            || registers
                .integral
                .is_some_and(|preset| self.channels.iter().sum::<u64>() >= preset)
    }

    fn advance(&mut self, seconds: f64) {
        if !self.counting || seconds <= 0.0 {
            return;
        }
        // In 20 ms ticks, the instrument's own clock resolution, so a time
        // preset stops on the tick that reaches it.
        let tick = 0.02;
        let mut left = seconds;
        while left > 1e-12 && self.counting {
            let step = left.min(tick);
            left -= step;
            self.real_time += step;
            self.live_time += step * (1.0 - self.dead_fraction);
            self.owed += self.counts_per_second * step * (1.0 - self.dead_fraction);
            let whole = self.owed.floor();
            self.owed -= whole;
            let whole = whole as u64;
            let length = self.channels.len();
            if length > 0 && whole > 0 {
                // Deterministic: three in four to a peak at mid-scale, the
                // rest a quarter of the way up.
                let peak = whole - whole / 4;
                if let Some(channel) = self.channels.get_mut(length / 2) {
                    *channel += peak;
                }
                if let Some(channel) = self.channels.get_mut(length / 4) {
                    *channel += whole / 4;
                }
            }
            if self.reached() {
                self.counting = false;
                let short = self.faults.iter().find_map(|fault| match fault {
                    Fault::ClockShortfall { seconds } => Some(*seconds),
                    _ => None,
                });
                if let (Some(short), Some(real)) = (short, self.registers.real)
                    && self.real_time >= real
                {
                    self.real_time = (real - short).max(0.0);
                }
                if let (Some(short), Some(live)) = (short, self.registers.live)
                    && self.live_time >= live
                {
                    self.live_time = (live - short).max(0.0);
                }
            }
        }
    }

    fn status_line(&mut self) -> String {
        let active = self.counting || self.lagging > 0;
        self.lagging = self.lagging.saturating_sub(1);
        let real = self.real_time;
        let live = self.live_time;
        let dead = self.dead_fraction * 100.0;
        let total: u64 = self.channels.iter().sum();
        let words = [
            ("RT", format!("{real:.2}")),
            ("LT", format!("{live:.2}")),
            ("DT", format!("{dead:.1}%")),
            ("ICR", format!("{:.0}", self.counts_per_second)),
            ("ACTIVE", u8::from(active).to_string()),
            ("TOTAL", total.to_string()),
        ];
        let mut dropped: Vec<String> = Vec::new();
        for fault in &mut self.faults {
            if let Fault::StatusWithout { key, count } = fault
                && *count > 0
            {
                *count -= 1;
                dropped.push(key.clone());
            }
        }
        words
            .iter()
            .filter(|(key, _)| !dropped.iter().any(|gone| gone == key))
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Carries out one command and says what the instrument answers.
    fn execute(&mut self, verb: &str, argument: &str) -> String {
        let number = argument
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite());
        let changing = !verb.starts_with("SHOW_");
        if changing && let Some(owner) = &self.locked_by {
            return format!("ERR locked by {owner}");
        }
        if verb.starts_with("SET_PRESET") && self.counting {
            self.violations
                .push(format!("{verb} {argument}").trim().to_string());
        }
        let set_time = |value: Option<f64>| value.filter(|seconds| *seconds > 0.0);
        let set_count = |value: Option<f64>| {
            value
                .filter(|count| *count >= 1.0)
                .map(|count| count as u64)
        };
        match verb {
            "SHOW_CONFIGURATION" => format!(
                "MODEL=RIG SERIAL=RIG-0001 FIRMWARE=rig-1 CHANNELS={}",
                self.channels.len()
            ),
            "SHOW_STATUS" => {
                let step = self.tick_per_status;
                self.advance(step);
                self.status_line()
            }
            "SHOW_DATA" => {
                let mut line = format!("DATA {}", self.channels.len());
                for count in &self.channels {
                    line.push(' ');
                    line.push_str(&count.to_string());
                }
                line
            }
            "SHOW_PRESETS" => format!(
                "PRESETS REAL={:.2} LIVE={:.2} PEAK={} INTEG={}",
                self.registers.real.unwrap_or(0.0),
                self.registers.live.unwrap_or(0.0),
                self.registers.peak.unwrap_or(0),
                self.registers.integral.unwrap_or(0)
            ),
            "START" => {
                self.start();
                "OK".into()
            }
            "STOP" => {
                if self.counting
                    && let Some(at) = self
                        .faults
                        .iter()
                        .position(|fault| matches!(fault, Fault::StopLag { .. }))
                    && let Fault::StopLag { reads } = self.faults.remove(at)
                {
                    self.lagging = reads;
                }
                self.counting = false;
                "OK".into()
            }
            "CLEAR" => {
                self.channels.iter_mut().for_each(|count| *count = 0);
                self.real_time = 0.0;
                self.live_time = 0.0;
                self.owed = 0.0;
                "OK".into()
            }
            "SET_PRESET_CLEAR" => {
                self.registers = Registers::default();
                "OK".into()
            }
            "SET_PRESET_REAL" | "SET_PRESET_LIVE" | "SET_PRESET_COUNT" | "SET_PRESET_INTEG" => {
                if number.is_none() {
                    return format!("ERR {argument:?} is not a number");
                }
                match verb {
                    "SET_PRESET_REAL" => self.registers.real = set_time(number),
                    "SET_PRESET_LIVE" => self.registers.live = set_time(number),
                    "SET_PRESET_COUNT" => self.registers.peak = set_count(number),
                    _ => self.registers.integral = set_count(number),
                }
                "OK".into()
            }
            "SET_HV" => match number {
                Some(volts) => {
                    self.high_voltage = volts;
                    "OK".into()
                }
                None => format!("ERR {argument:?} is not a number"),
            },
            "SET_GAIN_COARSE" => match number {
                Some(gain) => {
                    self.coarse_gain = gain;
                    "OK".into()
                }
                None => format!("ERR {argument:?} is not a number"),
            },
            "SET_GAIN_FINE" => "OK".into(),
            _ => format!("ERR unknown command {verb}"),
        }
    }

    fn answer(&mut self, command: &str) -> Result<String, DeviceError> {
        if self.faults.contains(&Fault::Disconnect) {
            return Err(DeviceError::Connection {
                address: "rig".into(),
                detail: "the line dropped".into(),
            });
        }
        let counting = self.counting;
        let (verb, argument) = match command.split_once(char::is_whitespace) {
            Some((verb, rest)) => (verb.to_ascii_uppercase(), rest.trim().to_string()),
            None => (command.to_ascii_uppercase(), String::new()),
        };
        // A scripted reply for this verb, if one is armed.
        let scripted = self.faults.iter_mut().find_map(|fault| match fault {
            Fault::Reply {
                verb: wanted,
                reply,
                count,
                execute,
            } if *count > 0 && wanted.eq_ignore_ascii_case(&verb) => {
                *count -= 1;
                Some((reply.clone(), *execute))
            }
            _ => None,
        });
        let mut reply = match scripted {
            Some((reply, true)) => {
                self.execute(&verb, &argument);
                reply
            }
            Some((reply, false)) => reply,
            None => self.execute(&verb, &argument),
        };
        // The slipped stream: this command is answered with the previous
        // command's reply, and its own reply waits for the next one.
        let slipping = self.faults.iter_mut().find_map(|fault| match fault {
            Fault::Slip { count } if *count > 0 => {
                *count -= 1;
                Some(())
            }
            _ => None,
        });
        if slipping.is_some() {
            let previous = self
                .behind
                .replace(reply)
                .or_else(|| self.log.last().map(|exchange| exchange.reply.clone()))
                .unwrap_or_else(|| "OK".into());
            reply = previous;
        } else {
            self.behind = None;
        }
        self.faults.retain(|fault| match fault {
            Fault::Reply { count, .. }
            | Fault::Slip { count }
            | Fault::StatusWithout { count, .. } => *count > 0,
            _ => true,
        });
        self.log.push(Exchange {
            command: command.to_string(),
            reply: reply.clone(),
            counting,
        });
        Ok(reply)
    }
}

/// The in-process line to a rig.
struct RigTransport {
    rig: Rig,
}

impl Transport for RigTransport {
    fn exchange(&mut self, command: &str) -> Result<String, DeviceError> {
        self.rig.answer(command)
    }

    fn peer(&self) -> String {
        "rig".into()
    }
}

#[cfg(test)]
mod tests {
    //! The rig's own faults, checked against the wire, so a rig test that
    //! passes cannot be passing because the fault never fired.
    use super::*;

    fn ask(line: &mut Box<dyn Transport>, command: &str) -> String {
        line.exchange(command).expect("exchange")
    }

    #[test]
    fn a_slipped_stream_answers_each_command_with_the_last_ones_reply() {
        let rig = Rig::new(4);
        let mut line = rig.transport();
        assert_eq!(ask(&mut line, "START"), "OK");
        rig.fault(Fault::Slip { count: 2 });
        assert_eq!(ask(&mut line, "SHOW_PRESETS"), "OK", "START's reply");
        assert!(
            ask(&mut line, "SHOW_STATUS").starts_with("PRESETS"),
            "SHOW_PRESETS's reply"
        );
        assert!(
            ask(&mut line, "SHOW_STATUS").starts_with("RT="),
            "back in step"
        );
    }

    #[test]
    fn a_lagging_stop_keeps_the_flag_up_for_its_reads_only() {
        let rig = Rig::new(4);
        let mut line = rig.transport();
        ask(&mut line, "START");
        rig.fault(Fault::StopLag { reads: 2 });
        ask(&mut line, "STOP");
        assert!(!rig.counting());
        assert!(ask(&mut line, "SHOW_STATUS").contains("ACTIVE=1"));
        assert!(ask(&mut line, "SHOW_STATUS").contains("ACTIVE=1"));
        assert!(ask(&mut line, "SHOW_STATUS").contains("ACTIVE=0"));
    }

    #[test]
    fn a_held_preset_stops_the_count_and_a_reached_one_blocks_start() {
        let rig = Rig::new(4);
        let mut line = rig.transport();
        ask(&mut line, "SET_PRESET_LIVE 1");
        ask(&mut line, "START");
        rig.tick(0.5);
        assert!(rig.counting());
        rig.tick(1.0);
        assert!(!rig.counting());
        assert!((rig.clocks().1 - 1.0).abs() < 1e-9, "{:?}", rig.clocks());
        assert_eq!(ask(&mut line, "START"), "OK");
        assert!(!rig.counting(), "the 926 accepts START and does not count");
    }

    #[test]
    fn replies_errors_status_words_locks_and_drops() {
        let rig = Rig::new(4);
        let mut line = rig.transport();
        rig.err("STOP", "busy", 1);
        assert_eq!(ask(&mut line, "STOP"), "ERR busy");
        assert_eq!(ask(&mut line, "STOP"), "OK");
        rig.fault(Fault::StatusWithout {
            key: "ACTIVE".into(),
            count: 1,
        });
        assert!(!ask(&mut line, "SHOW_STATUS").contains("ACTIVE"));
        assert!(ask(&mut line, "SHOW_STATUS").contains("ACTIVE"));
        rig.lock_elsewhere("MAESTRO");
        assert_eq!(ask(&mut line, "START"), "ERR locked by MAESTRO");
        assert!(ask(&mut line, "SHOW_STATUS").starts_with("RT="));
        rig.unlock_elsewhere();
        ask(&mut line, "START");
        ask(&mut line, "SET_PRESET_LIVE 5");
        assert_eq!(rig.violations(), vec!["SET_PRESET_LIVE 5"]);
        rig.fault(Fault::Disconnect);
        assert!(line.exchange("SHOW_STATUS").is_err());
    }
}
