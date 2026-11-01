//! Turning an argument vector into a command line you can paste into a Windows
//! terminal. The app itself never goes through a shell (it passes an argv), so
//! this is only for display and "Copy to clipboard".
//!
//! Two layers of quoting are involved on Windows:
//! 1. **Program argv parsing** (MSVC CRT / `CommandLineToArgvW`, which ffmpeg uses):
//!    an argument containing spaces, tabs or `"` is wrapped in `"…"`, embedded `"`
//!    become `\"`, and backslashes are doubled only where they precede a `"`.
//! 2. **The shell**:
//!    * **cmd.exe** – `& | < > ^ ( )` are special outside quotes. Any argument
//!      containing them is quoted; if an embedded `\"` would leave cmd "outside"
//!      quotes, those characters are additionally escaped with `^`.
//!    * **PowerShell** – the program is invoked with the call operator
//!      `& '…\ffmpeg.exe'` and every argument that isn't plainly safe is a
//!      single-quoted literal (`'` doubled, also the curly ‘ ’ ‚ ‛ variants), so
//!      `$`, `` ` ``, `,`, `;`, `@`, `{}` … are never interpreted. PowerShell then builds
//!      the program's command line itself (layer 1). Arguments that contain `"` are
//!      passed correctly by PowerShell 7.3+; Windows PowerShell 5.1 mangles embedded
//!      double quotes in native-command arguments (a PowerShell limitation).
//!      Dash-led tokens other than plain `-name` (e.g. `-c:v`, `-metadata:s:a:0`) are
//!      quoted too, because PowerShell's parameter parsing can split them. An argument
//!      that is exactly `--%` can't be passed by PowerShell at all (ffmpeg never needs one).

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Shell {
    #[default]
    PowerShell,
    Cmd,
}

impl Shell {
    pub const ALL: [Shell; 2] = [Shell::PowerShell, Shell::Cmd];
    pub fn label(self) -> &'static str {
        match self {
            Shell::PowerShell => "PowerShell",
            Shell::Cmd => "Command Prompt (cmd)",
        }
    }
    /// Line separator for several commands.
    pub fn newline(self) -> &'static str {
        "\r\n"
    }
}

const CMD_META: &[char] = &['&', '|', '<', '>', '^', '(', ')'];

/// Quote one argument for the program's own argv parser (CRT rules).
pub fn crt_quote(arg: &str) -> String {
    crt_quote_if(arg, false)
}

fn crt_quote_if(arg: &str, force: bool) -> String {
    let needs = force || arg.is_empty() || arg.chars().any(|c| matches!(c, ' ' | '\t' | '\n' | '\x0b' | '"'));
    if !needs {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut bs = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => bs += 1,
            '"' => {
                out.extend(std::iter::repeat('\\').take(bs * 2 + 1));
                out.push('"');
                bs = 0;
            }
            _ => {
                out.extend(std::iter::repeat('\\').take(bs));
                out.push(c);
                bs = 0;
            }
        }
    }
    out.extend(std::iter::repeat('\\').take(bs * 2));
    out.push('"');
    out
}

/// `^`-escape cmd metacharacters that cmd would see outside of quotes
/// (cmd toggles its quote state on every `"`, it doesn't know `\"`).
fn cmd_caret(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_q = false;
    for c in s.chars() {
        if c == '"' {
            in_q = !in_q;
        } else if !in_q && CMD_META.contains(&c) {
            out.push('^');
        }
        out.push(c);
    }
    out
}

/// Program path for cmd: quoted when it has spaces or metacharacters (no escapes in argv[0]).
fn cmd_program(p: &str) -> String {
    if p.is_empty() || p.chars().any(|c| c.is_whitespace() || CMD_META.contains(&c)) {
        format!("\"{p}\"")
    } else {
        p.to_string()
    }
}

/// One line for Command Prompt.
pub fn cmd_line(program: &Path, args: &[String]) -> String {
    let mut out = cmd_program(&program.to_string_lossy());
    for a in args {
        out.push(' ');
        let force = a.chars().any(|c| CMD_META.contains(&c));
        out.push_str(&crt_quote_if(a, force));
    }
    // cmd's quote state runs across arguments (an embedded \" flips it), so the
    // caret pass must see the whole line
    cmd_caret(&out)
}

fn ps_safe(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    if s.starts_with('-') {
        // PowerShell parses dash tokens as parameters: `-c:v` / `-foo:bar` / `-x.y` can be
        // split or dropped depending on context, so only plain `-name` stays bare
        return s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    }
    s.chars().all(|c| c.is_ascii_alphanumeric() || "_+./:=\\".contains(c))
}

/// A PowerShell single-quoted literal.
pub fn ps_quote(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// One line for PowerShell: `& 'C:\…\ffmpeg.exe' -i 'C:\my video.mkv' …`.
pub fn ps_line(program: &Path, args: &[String]) -> String {
    let mut out = format!("& {}", ps_quote(&program.to_string_lossy()));
    for a in args {
        out.push(' ');
        out.push_str(&if ps_safe(a) { a.clone() } else { ps_quote(a) });
    }
    out
}

pub fn command_line(shell: Shell, program: &Path, args: &[String]) -> String {
    match shell {
        Shell::PowerShell => ps_line(program, args),
        Shell::Cmd => cmd_line(program, args),
    }
}

/// PowerShell statement writing `contents` to `path` as UTF-8 without BOM with LF line
/// endings (what the app writes for chapter files), in both PowerShell 5.1 and 7.
pub fn ps_write_file(path: &Path, contents: &str) -> String {
    let lines: Vec<&str> = contents.split('\n').collect();
    let (body, trailing_nl) = match lines.split_last() {
        Some((last, rest)) if last.is_empty() => (rest.to_vec(), true),
        _ => (lines.clone(), false),
    };
    let list = body.iter().map(|l| ps_quote(l)).collect::<Vec<_>>().join(", ");
    let nl = if trailing_nl { " + \"`n\"" } else { "" };
    format!("[IO.File]::WriteAllText({}, (@({list}) -join \"`n\"){nl})", ps_quote(&path.to_string_lossy()))
}

/// Everything needed to run one job by hand: for PowerShell the helper files
/// (e.g. the temp chapter file) are written first; cmd gets the command only.
pub fn job_script(shell: Shell, program: &Path, args: &[String], side_files: &[(std::path::PathBuf, String)]) -> String {
    let mut out = String::new();
    if shell == Shell::PowerShell {
        for (p, c) in side_files {
            out.push_str(&ps_write_file(p, c));
            out.push_str(shell.newline());
        }
    }
    out.push_str(&command_line(shell, program, args));
    out
}

/// The step after ffmpeg that the app performs itself: move the finished temporary file
/// to its final name, only if ffmpeg succeeded. Without `overwrite` an existing file is
/// never replaced.
pub fn publish_line(shell: Shell, temp: &Path, dest: &Path, overwrite: bool) -> String {
    let (t, d) = (temp.to_string_lossy(), dest.to_string_lossy());
    match shell {
        Shell::PowerShell => {
            let (t, d) = (ps_quote(&t), ps_quote(&d));
            if overwrite {
                format!("if ($LASTEXITCODE -eq 0) {{ if ([IO.File]::Exists({d})) {{ [IO.File]::Replace({t}, {d}, $null) }} else {{ [IO.File]::Move({t}, {d}) }} }}")
            } else {
                format!("if ($LASTEXITCODE -eq 0) {{ [IO.File]::Move({t}, {d}) }}")
            }
        }
        Shell::Cmd => {
            if overwrite {
                format!("if not errorlevel 1 move /Y \"{t}\" \"{d}\" >nul")
            } else {
                format!("if not errorlevel 1 if not exist \"{d}\" move \"{t}\" \"{d}\" >nul")
            }
        }
    }
}

/// Split a command line the way the MSVC CRT / `CommandLineToArgvW` does
/// (argv\[1..\] rules). Used to verify the quoting.
pub fn parse_crt(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    loop {
        while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let mut arg = String::new();
        let mut in_q = false;
        while i < chars.len() {
            let mut bs = 0;
            while i < chars.len() && chars[i] == '\\' {
                bs += 1;
                i += 1;
            }
            if i < chars.len() && chars[i] == '"' {
                arg.extend(std::iter::repeat('\\').take(bs / 2));
                if bs % 2 == 1 {
                    arg.push('"');
                } else if in_q && i + 1 < chars.len() && chars[i + 1] == '"' {
                    arg.push('"');
                    i += 1;
                } else {
                    in_q = !in_q;
                }
                i += 1;
                continue;
            }
            arg.extend(std::iter::repeat('\\').take(bs));
            if i >= chars.len() {
                break;
            }
            let c = chars[i];
            if (c == ' ' || c == '\t') && !in_q {
                break;
            }
            arg.push(c);
            i += 1;
        }
        out.push(arg);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    /// What cmd hands to the program: carets outside quotes removed.
    fn cmd_unescape(s: &str) -> String {
        let mut out = String::new();
        let mut in_q = false;
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            if c == '"' {
                in_q = !in_q;
            } else if c == '^' && !in_q {
                if let Some(n) = it.next() {
                    out.push(n);
                }
                continue;
            }
            out.push(c);
        }
        out
    }

    /// cmd must never see an unescaped metacharacter outside quotes.
    fn cmd_safe(s: &str) -> bool {
        let mut in_q = false;
        let mut esc = false;
        for c in s.chars() {
            if esc {
                esc = false;
                continue;
            }
            if c == '"' {
                in_q = !in_q;
            } else if !in_q && c == '^' {
                esc = true;
            } else if !in_q && CMD_META.contains(&c) {
                return false;
            }
        }
        true
    }

    fn nasty() -> Vec<String> {
        v(&[
            "-i",
            r"C:\Users\Me\My Videos\game (1).mkv",
            "-f",
            "ffmetadata",
            r"C:\Users\Me\AppData\Local\Temp\ffenc_chapters_1.ffmeta",
            "-vf",
            "scale=1280:-2,format=yuv420p",
            "-map",
            "0:a?",
            "-metadata",
            "title=Tom & Jerry \"Live\" <1> | 50% ^ 'x' $HOME `t` ; #a",
            "-metadata",
            r"comment=ends with backslash\",
            r"D:\out dir\",
            "",
            "ünï 視頻",
            "a\\\\\"b",
            "x&y (odd quote count before: cmd state carries over)",
            "plain",
        ])
    }

    #[test]
    fn publish_lines() {
        let (t, d) = (Path::new(r"C:\o\it's.ffenc-partial-1.mp4"), Path::new(r"C:\o\it's.mp4"));
        assert_eq!(publish_line(Shell::PowerShell, t, d, false), r"if ($LASTEXITCODE -eq 0) { [IO.File]::Move('C:\o\it''s.ffenc-partial-1.mp4', 'C:\o\it''s.mp4') }");
        assert!(publish_line(Shell::PowerShell, t, d, true).contains("[IO.File]::Replace("));
        assert_eq!(publish_line(Shell::Cmd, t, d, false), r#"if not errorlevel 1 if not exist "C:\o\it's.mp4" move "C:\o\it's.ffenc-partial-1.mp4" "C:\o\it's.mp4" >nul"#);
        assert_eq!(publish_line(Shell::Cmd, t, d, true), r#"if not errorlevel 1 move /Y "C:\o\it's.ffenc-partial-1.mp4" "C:\o\it's.mp4" >nul"#);
    }

    #[test]
    fn crt_rules() {
        assert_eq!(crt_quote("plain"), "plain");
        assert_eq!(crt_quote(r"C:\a b\c.mkv"), r#""C:\a b\c.mkv""#);
        assert_eq!(crt_quote(r"C:\dir\"), r"C:\dir\", "no quoting needed, backslash kept");
        assert_eq!(crt_quote(r"C:\my dir\"), r#""C:\my dir\\""#, "trailing backslash doubled before the closing quote");
        assert_eq!(crt_quote(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(crt_quote(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(crt_quote(""), "\"\"");
        let a = nasty();
        let line = a.iter().map(|s| crt_quote(s)).collect::<Vec<_>>().join(" ");
        assert_eq!(parse_crt(&line), a, "round trip through the CRT parser");
    }

    #[test]
    fn cmd_round_trip_and_safety() {
        let prog = Path::new(r"C:\Program Files\FFmpeg\bin\ffmpeg.exe");
        let a = nasty();
        let line = cmd_line(prog, &a);
        assert!(line.starts_with(r#""C:\Program Files\FFmpeg\bin\ffmpeg.exe" -i "C:\Users\Me\My Videos\game (1).mkv" -f ffmetadata"#), "{line}");
        assert!(cmd_safe(&line), "{line}");
        let seen = cmd_unescape(&line);
        let mut parsed = parse_crt(&seen);
        assert_eq!(parsed.remove(0), prog.to_string_lossy());
        assert_eq!(parsed, a);
        // simple args stay readable
        assert_eq!(cmd_line(Path::new(r"C:\ff\ffmpeg.exe"), &v(&["-y", "-i", "a.mkv"])), r"C:\ff\ffmpeg.exe -y -i a.mkv");
        // metacharacter without spaces is quoted, not caret-escaped
        assert_eq!(cmd_line(Path::new("ffmpeg"), &v(&["a&b", "x(1)"])), r#"ffmpeg "a&b" "x(1)""#);
        // embedded quote: the part cmd sees as unquoted gets carets
        let l = cmd_line(Path::new("ffmpeg"), &v(&[r#"t="A & B""#]));
        assert_eq!(l, r#"ffmpeg "t=\"A ^& B\"""#);
        assert!(cmd_safe(&l));
        assert_eq!(parse_crt(&cmd_unescape(&l))[1], r#"t="A & B""#);
    }

    /// Minimal PowerShell tokenizer for the subset ps_line emits: barewords and '…' literals.
    fn ps_args(line: &str) -> (String, Vec<String>) {
        let rest = line.strip_prefix("& ").expect("call operator");
        let chars: Vec<char> = rest.chars().collect();
        let mut out = vec![];
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == ' ' {
                i += 1;
                continue;
            }
            let mut s = String::new();
            if chars[i] == '\'' {
                i += 1;
                loop {
                    let c = chars[i];
                    let q = matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}');
                    if q && i + 1 < chars.len() && chars[i + 1] == c {
                        s.push(c);
                        i += 2;
                    } else if q {
                        i += 1;
                        break;
                    } else {
                        s.push(c);
                        i += 1;
                    }
                }
            } else {
                let dash = chars[i] == '-';
                while i < chars.len() && chars[i] != ' ' {
                    let ok = if dash { chars[i].is_ascii_alphanumeric() || "_-".contains(chars[i]) } else { chars[i].is_ascii_alphanumeric() || "_+./:=\\".contains(chars[i]) };
                    assert!(ok, "unsafe bareword char {:?} in {line}", chars[i]);
                    s.push(chars[i]);
                    i += 1;
                }
            }
            out.push(s);
        }
        let prog = out.remove(0);
        (prog, out)
    }

    #[test]
    fn powershell_literals() {
        let prog = Path::new(r"C:\Users\O'Brien\ffmpeg\bin\ffmpeg.exe");
        let mut a = nasty();
        a.pop(); // keep "plain" check below
        a.push("it’s".into()); // curly apostrophe is a quote in PowerShell
        a.push("@splat".into());
        a.push("-c:v".into());
        a.push("-x.y".into());
        a.push("{x},$y".into());
        let line = ps_line(prog, &a);
        assert!(line.starts_with(r"& 'C:\Users\O''Brien\ffmpeg\bin\ffmpeg.exe' -i 'C:\Users\Me\My Videos\game (1).mkv' -f ffmetadata"), "{line}");
        assert!(line.contains(" -vf 'scale=1280:-2,format=yuv420p' "), "comma would make an array");
        assert!(line.contains(" -map '0:a?' "));
        assert!(line.contains("'it’’s'"));
        let (p, parsed) = ps_args(&line);
        assert_eq!(p, prog.to_string_lossy());
        assert_eq!(parsed, a);
        assert_eq!(ps_line(Path::new("ffmpeg.exe"), &v(&["-y", "-c:v", "libx264", "-crf", "22", "-map_chapters", "1", "out.mp4"])), "& 'ffmpeg.exe' -y '-c:v' libx264 -crf 22 -map_chapters 1 out.mp4");
    }

    #[test]
    fn script_with_chapter_file() {
        let chap = ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1000\ntitle=it's \\= ü\n";
        let side = vec![(std::path::PathBuf::from(r"C:\Temp\ffenc_chapters_1.ffmeta"), chap.to_string())];
        let args = v(&["-f", "ffmetadata", "-i", r"C:\Temp\ffenc_chapters_1.ffmeta"]);
        let ps = job_script(Shell::PowerShell, Path::new(r"C:\ff\ffmpeg.exe"), &args, &side);
        let mut lines = ps.split("\r\n");
        assert_eq!(
            lines.next().unwrap(),
            r#"[IO.File]::WriteAllText('C:\Temp\ffenc_chapters_1.ffmeta', (@(';FFMETADATA1', '[CHAPTER]', 'TIMEBASE=1/1000', 'START=0', 'END=1000', 'title=it''s \= ü') -join "`n") + "`n")"#
        );
        assert_eq!(lines.next().unwrap(), r"& 'C:\ff\ffmpeg.exe' -f ffmetadata -i C:\Temp\ffenc_chapters_1.ffmeta");
        assert!(lines.next().is_none());
        let cmd = job_script(Shell::Cmd, Path::new(r"C:\ff\ffmpeg.exe"), &args, &side);
        assert_eq!(cmd, r"C:\ff\ffmpeg.exe -f ffmetadata -i C:\Temp\ffenc_chapters_1.ffmeta");
        assert!(ps_write_file(Path::new("x"), "a\nb").ends_with("-join \"`n\"))"), "no trailing newline added");
    }
}
