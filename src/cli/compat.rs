use std::ffi::{OsStr, OsString};

use super::Cli;

enum Adjustment {
    None,
    JoinValue,
    StartTrailing,
    RejectValue(String),
}

/// Preserve the argv behavior of the original Clap CLI where Usage 6.12 differs.
/// Resolve flag ownership and positional context through Usage's own tables.
pub(super) fn prepare_args(args: &mut Vec<OsString>) -> Result<(), String> {
    let mut index = 0;
    while index < args.len() {
        let words: Vec<_> = args.iter().map(OsString::as_os_str).collect();
        match adjustment(&words, index) {
            Adjustment::None => index += 1,
            Adjustment::JoinValue => {
                let value = args.remove(index);
                if args[index - 1].as_encoded_bytes().starts_with(b"--") {
                    args[index - 1].push("=");
                }
                args[index - 1].push(value);
            }
            Adjustment::StartTrailing => {
                args.insert(index, OsString::from("--"));
                break;
            }
            Adjustment::RejectValue(message) => return Err(message),
        }
    }
    Ok(())
}

fn at_clone_tail(parser: &usage::Parser<'_, '_, '_>) -> bool {
    parser.command().name == "clone" && parser.pending_arg().is_some_and(|arg| arg.name == "EXTRA")
}

fn has_unknown_flag(words: &[&OsStr]) -> bool {
    let mut parser = usage::Parser::new(Cli::command(), words);
    while let Some(event) = parser.next_event() {
        if let Err(error) = event {
            return matches!(error, usage::Error::UnknownFlag { .. });
        }
    }
    false
}

fn adjustment(words: &[&OsStr], index: usize) -> Adjustment {
    let token = words[index].as_encoded_bytes();
    if !token.starts_with(b"-") {
        return Adjustment::None;
    }
    let mut parser = usage::Parser::new(Cli::command(), &words[..index]);
    while let Some(event) = parser.next_event() {
        if let Err(error) = event {
            // After clone's remote and subdir, Clap accepts an unrecognized
            // flag-like word as an option value. Join it to that option so
            // Usage binds it unambiguously. Recognized flags remain errors.
            if matches!(error, usage::Error::MissingFlagValue { .. }) && at_clone_tail(&parser) {
                let mut probe = words[..index - 1].to_vec();
                probe.push(words[index]);
                if has_unknown_flag(&probe) {
                    return Adjustment::JoinValue;
                }
            }
            return Adjustment::None;
        }
    }
    if parser.flags_stopped() {
        return Adjustment::None;
    }

    // Usage ignores `=value` on a boolean switch, even `--force=false`.
    // Reject it before dispatch, using the flags in scope at this position.
    if let Some(body) = token.strip_prefix(b"--")
        && let Some(equal) = body.iter().position(|byte| *byte == b'=')
        && parser.flags_in_scope().any(|flag| {
            !flag.takes_value
                && flag
                    .longs
                    .iter()
                    .any(|name| name.as_bytes() == &body[..equal])
        })
    {
        return Adjustment::RejectValue(format!(
            "error: unexpected value '{}' for '--{}' found; no more were expected",
            String::from_utf8_lossy(&body[equal + 1..]),
            String::from_utf8_lossy(&body[..equal]),
        ));
    }

    // A first unknown flag at clone's trailing positional starts `extra`,
    // whose entire remainder is reported by the existing command handler.
    if at_clone_tail(&parser) && has_unknown_flag(&words[..=index]) {
        return Adjustment::StartTrailing;
    }
    Adjustment::None
}
