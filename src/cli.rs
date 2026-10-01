use std::{ffi::OsString, path::PathBuf};

#[derive(Debug, PartialEq)]
pub enum Command {
    Launch(Option<PathBuf>),
    Version,
    Help,
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, &'static str> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Ok(Command::Launch(None));
    };
    let command = if first == "--" {
        Command::Launch(args.next().map(PathBuf::from))
    } else if first == "--version" || first == "-V" {
        Command::Version
    } else if first == "--help" || first == "-h" {
        Command::Help
    } else if first.to_string_lossy().starts_with('-') {
        return Err("unknown option");
    } else {
        Command::Launch(Some(PathBuf::from(first)))
    };
    if args.next().is_some() {
        return Err("expected at most one directory");
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_and_unicode_directories_are_unambiguous() {
        let parse_args = |args: &[&str]| parse(args.iter().map(OsString::from));
        assert_eq!(parse_args(&[]), Ok(Command::Launch(None)));
        assert_eq!(parse_args(&["--version"]), Ok(Command::Version));
        assert_eq!(parse_args(&["--help"]), Ok(Command::Help));
        assert_eq!(
            parse_args(&["Çalışma alanı"]),
            Ok(Command::Launch(Some(PathBuf::from("Çalışma alanı"))))
        );
        assert_eq!(
            parse_args(&["--", "--version"]),
            Ok(Command::Launch(Some(PathBuf::from("--version"))))
        );
        assert!(parse_args(&["--unknown"]).is_err());
        assert!(parse_args(&["--version", "extra"]).is_err());
        assert!(parse_args(&["one", "two"]).is_err());
    }
}
