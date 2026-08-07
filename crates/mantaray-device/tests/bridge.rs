//! The bridge transport against a stand-in helper.
//!
//! The helper is a shell script rather than the real `mantaray-mcb`, because
//! what is under test is the transport's own bookkeeping - that a question and
//! its answer stay together - and no instrument is needed to ask that.

#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use mantaray_device::BridgeTransport;
use mantaray_device::transport::Transport;

/// The helper scripts, written once before any test runs.
///
/// All of them, up front: writing one while another test is spawning fails
/// that spawn with ETXTBSY, because the writing thread's descriptor is still
/// open in the same process when the child reaches `exec`.
fn helper(name: &str) -> PathBuf {
    static DIRECTORY: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIRECTORY
        .get_or_init(|| {
            let directory =
                std::env::temp_dir().join(format!("mantaray-bridge-{}", std::process::id()));
            std::fs::create_dir_all(&directory).expect("the helper directory is made");
            for (name, body) in [
                ("stalls-once", STALLS_ONCE),
                ("echoes", ECHOES),
                ("dies", DIES),
            ] {
                let path = directory.join(name);
                let mut file = std::fs::File::create(&path).expect("the helper is written");
                write!(file, "#!/bin/sh\n{body}").expect("the helper is written");
                drop(file);
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                    .expect("the helper is runnable");
            }
            directory
        })
        .join(name)
}

/// A helper that stalls past the exchange timeout on its first command and
/// then answers normally, so the abandoned command's reply arrives late.
const STALLS_ONCE: &str = "\
first=1
while IFS= read -r line; do
    if [ \"$first\" = 1 ]; then
        first=0
        sleep 11
    fi
    echo \"REPLY-TO $line\"
done
";

/// A helper that answers every command, as a working one does.
const ECHOES: &str = "while IFS= read -r line; do echo \"REPLY-TO $line\"; done\n";

/// A helper that is gone before the first command reaches it.
const DIES: &str = "exit 0\n";

#[test]
fn a_late_reply_is_never_handed_to_the_command_after_it() {
    // The bug this pins: a timed-out exchange left its reply in the channel,
    // so the next command read it as its own and every reading from then on
    // was the previous one's. Silently: a status poll that lands on a DATA
    // line parses as no counts and not counting, which is what an idle
    // instrument looks like.
    let mut bridge =
        BridgeTransport::start(&helper("stalls-once"), 1, None).expect("the helper starts");

    let first = bridge.exchange("ONE");
    assert!(first.is_err(), "the stall should time the first one out");

    assert_eq!(
        bridge.exchange("TWO").expect("the second one answers"),
        "REPLY-TO TWO",
        "the second command must get its own reply, not the first's"
    );
    assert_eq!(
        bridge.exchange("THREE").expect("the third one answers"),
        "REPLY-TO THREE",
        "and the alignment must hold afterwards"
    );
}

#[test]
fn an_ordinary_exchange_answers_the_command_it_was_given() {
    let mut bridge = BridgeTransport::start(&helper("echoes"), 3, None).expect("the helper starts");

    assert_eq!(
        bridge.exchange("SHOW_STATUS").unwrap(),
        "REPLY-TO SHOW_STATUS"
    );
    assert_eq!(bridge.exchange("SHOW_DATA").unwrap(), "REPLY-TO SHOW_DATA");
    assert!(bridge.peer().contains("detector 3"));
}

#[test]
fn a_helper_that_dies_is_reported_rather_than_waited_on_forever() {
    let mut bridge = BridgeTransport::start(&helper("dies"), 1, None).expect("the helper starts");

    let outcome = bridge.exchange("SHOW_STATUS");
    assert!(outcome.is_err(), "a dead helper cannot answer: {outcome:?}");
}
