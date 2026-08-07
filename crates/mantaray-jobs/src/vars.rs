//! `.JOB` file variables (MAESTRO §6.3).
//!
//! `$(Name)` expansions and the `???` loop-counter substitution. Path splitting
//! accepts both separators so a job written on Windows also runs on Linux.

/// The variables a job can refer to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JobVariables {
    spectrum_path: Option<String>,
    /// Directory the program lives in.
    pub mca_dir: String,
    /// Directory the program started in.
    pub current_dir: String,
    /// Value entered by `ASK_PASSWORD`.
    pub password: String,
    /// Owner entered by `ASK_PASSWORD`.
    pub owner: String,
    loop_index: u32,
}

impl JobVariables {
    /// Empty variables.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the file a read operation just used.
    ///
    /// `LOAD`, `RECALL`, `RECALL_CALIB`, `RECALL_ROI` and `STRIP` all update it.
    pub fn set_spectrum_path(&mut self, path: &str) {
        self.spectrum_path = Some(path.to_string());
    }

    /// The current spectrum path, if any.
    pub fn spectrum_path(&self) -> Option<&str> {
        self.spectrum_path.as_deref()
    }

    /// Sets the zero-based loop counter.
    pub fn set_loop(&mut self, index: u32) {
        self.loop_index = index;
    }

    /// The zero-based loop counter.
    pub fn loop_index(&self) -> u32 {
        self.loop_index
    }

    /// Looks a variable up by name, case-insensitively.
    pub fn get(&self, name: &str) -> Option<String> {
        let path = self.spectrum_path.as_deref().unwrap_or("");
        let (directory, file) = split_path(path);
        let (stem, extension) = split_extension(file);
        let value = match name.to_ascii_lowercase().as_str() {
            "fullpath" => path.to_string(),
            "fullbase" => {
                if directory.is_empty() {
                    stem.to_string()
                } else {
                    format!("{directory}{}{stem}", separator_of(path))
                }
            }
            "fileext" => extension.to_string(),
            "filedir" => directory.to_string(),
            "mcadir" => self.mca_dir.clone(),
            "curdir" => self.current_dir.clone(),
            "loop" => self.loop_index.to_string(),
            "loop1" => (self.loop_index + 1).to_string(),
            "bel" => "\u{7}".to_string(),
            "cr" => "\r".to_string(),
            "ff" => "\u{c}".to_string(),
            "lf" => "\n".to_string(),
            "esc" => "\u{1b}".to_string(),
            "shortpath" => file.to_string(),
            "shortbase" => stem.to_string(),
            "password" => self.password.clone(),
            "owner" => self.owner.clone(),
            _ => return None,
        };
        Some(value)
    }

    /// Expands `$(Name)` references and `???` in a string.
    ///
    /// Unknown names are left untouched, so a stray `$(` cannot break a job.
    pub fn expand(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let bytes: Vec<char> = text.chars().collect();
        let mut index = 0;
        while index < bytes.len() {
            // Loop counter substitution, three question marks.
            if bytes[index] == '?'
                && bytes.get(index + 1) == Some(&'?')
                && bytes.get(index + 2) == Some(&'?')
            {
                out.push_str(&format!("{:03}", self.loop_index + 1));
                index += 3;
                continue;
            }
            if bytes[index] == '$'
                && bytes.get(index + 1) == Some(&'(')
                && let Some(close) = (index + 2..bytes.len()).find(|i| bytes[*i] == ')')
            {
                let name: String = bytes[index + 2..close].iter().collect();
                match self.get(&name) {
                    Some(value) => out.push_str(&value),
                    // A reference to something unset is left as it was written.
                    None => out.push_str(&format!("$({name})")),
                }
                index = close + 1;
                continue;
            }
            out.push(bytes[index]);
            index += 1;
        }
        out
    }
}

/// Splits a path into directory and file name, accepting `\` and `/`.
fn split_path(path: &str) -> (&str, &str) {
    match path.rfind(['\\', '/']) {
        Some(index) => (&path[..index], &path[index + 1..]),
        None => ("", path),
    }
}

/// Splits a file name into stem and extension.
fn split_extension(file: &str) -> (&str, &str) {
    match file.rfind('.') {
        Some(index) if index > 0 => (&file[..index], &file[index + 1..]),
        _ => (file, ""),
    }
}

/// Which separator a path uses, defaulting to the platform's.
fn separator_of(path: &str) -> char {
    if path.contains('\\') { '\\' } else { '/' }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> JobVariables {
        let mut vars = JobVariables::new();
        vars.set_spectrum_path("D:\\USER\\SOIL\\SAM001.SPC");
        vars
    }

    #[test]
    fn the_manual_example_expands_as_documented() {
        let vars = vars();
        assert_eq!(vars.get("FullPath").unwrap(), "D:\\USER\\SOIL\\SAM001.SPC");
        assert_eq!(vars.get("FullBase").unwrap(), "D:\\USER\\SOIL\\SAM001");
        assert_eq!(vars.get("FileExt").unwrap(), "SPC");
        assert_eq!(vars.get("FileDir").unwrap(), "D:\\USER\\SOIL");
        assert_eq!(vars.get("ShortPath").unwrap(), "SAM001.SPC");
        assert_eq!(vars.get("ShortBase").unwrap(), "SAM001");
    }

    #[test]
    fn unix_paths_split_too() {
        let mut vars = JobVariables::new();
        vars.set_spectrum_path("/home/user/soil/sam001.spe");
        assert_eq!(vars.get("FileDir").unwrap(), "/home/user/soil");
        assert_eq!(vars.get("ShortBase").unwrap(), "sam001");
        assert_eq!(vars.get("FullBase").unwrap(), "/home/user/soil/sam001");
    }

    #[test]
    fn names_are_case_insensitive() {
        assert_eq!(vars().get("SHORTBASE"), vars().get("shortbase"));
    }

    #[test]
    fn question_marks_use_a_one_based_counter() {
        let mut vars = JobVariables::new();
        assert_eq!(vars.expand("DECAY???.CHN"), "DECAY001.CHN");
        vars.set_loop(11);
        assert_eq!(vars.expand("DECAY???.CHN"), "DECAY012.CHN");
    }

    #[test]
    fn an_unterminated_reference_is_left_alone() {
        assert_eq!(vars().expand("$(FullPath"), "$(FullPath");
        assert_eq!(vars().expand("cost $5 (each)"), "cost $5 (each)");
    }
}
