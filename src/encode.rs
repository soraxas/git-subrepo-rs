/// Encode a subdir path as a valid git ref component.
/// Returns the encoded string (same as input for simple paths).
pub fn encode_subdir(subdir: &str) -> String {
    // Check if already valid
    let ok = std::process::Command::new("git")
        .args(["check-ref-format", &format!("subrepo/{subdir}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok {
        return subdir.to_string();
    }

    let mut s = subdir.to_string();

    // 0. escape %
    s = s.replace('%', "%25");

    // 1. No slash-separated component can begin with '.' or end with '.lock'
    let parts: Vec<String> = s
        .split('/')
        .map(|p| {
            let mut p = p.to_string();
            if p.starts_with('.') {
                p = format!("%2e{}", &p[1..]);
            }
            if p.ends_with(".lock") {
                p = format!("{}%2elock", &p[..p.len() - 5]);
            }
            p
        })
        .collect();
    s = parts.join("/");

    // 3. No two consecutive dots
    while s.contains("..") {
        s = s.replace("..", "%2e%2e");
    }

    // 4 & 5. Encode control chars, space, ~, ^, :, ?, *, [
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        let code = c as u32;
        if code < 0x20 {
            result.push_str(&format!("%{:02x}", code));
        } else {
            match c {
                '\x7f' => result.push_str("%7f"),
                ' ' => result.push_str("%20"),
                '~' => result.push_str("%7e"),
                '^' => result.push_str("%5e"),
                ':' => result.push_str("%3a"),
                '?' => result.push_str("%3f"),
                '*' => result.push_str("%2a"),
                '[' => result.push_str("%5b"),
                '\n' => result.push_str("%0a"),
                '\\' => result.push_str("%5c"),
                _ => result.push(c),
            }
        }
    }
    s = result;

    // 6. No consecutive slashes
    while s.contains("//") {
        s = s.replace("//", "/");
    }

    // 7. Cannot end with a dot
    if s.ends_with('.') {
        s = format!("{}%2e", &s[..s.len() - 1]);
    }

    // 8. Cannot contain @{
    s = s.replace("@{", "%40{");

    // 10. Cannot contain backslash (already handled above)

    s
}
