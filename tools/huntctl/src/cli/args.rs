use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;

pub(crate) fn option(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

pub(crate) fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|argument| argument == name)
}

pub(crate) fn repeated_option(args: &[String], name: &str) -> Vec<String> {
    args.windows(2)
        .filter(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
        .collect()
}

pub(crate) fn required_path(args: &[String], name: &str) -> Result<PathBuf, Box<dyn Error>> {
    option(args, name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing required {name} <path>").into())
}

pub(crate) fn usize_option(
    args: &[String],
    name: &str,
    default: usize,
) -> Result<usize, Box<dyn Error>> {
    Ok(option(args, name)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(default))
}

pub(crate) fn u64_option(args: &[String], name: &str, default: u64) -> Result<u64, Box<dyn Error>> {
    Ok(option(args, name)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(default))
}

pub(crate) fn u32_option(args: &[String], name: &str, default: u32) -> Result<u32, Box<dyn Error>> {
    Ok(option(args, name)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(default))
}

pub(crate) fn timeout_option(args: &[String]) -> Result<Duration, Box<dyn Error>> {
    if let Some(milliseconds) = option(args, "--timeout-ms") {
        return Ok(Duration::from_millis(milliseconds.parse()?));
    }
    Ok(Duration::from_secs(
        option(args, "--timeout-seconds")
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or(300),
    ))
}
