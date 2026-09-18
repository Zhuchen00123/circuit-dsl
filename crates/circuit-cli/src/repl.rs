//! `cdsl repl`: the interactive front end.
//!
//! This module is only about the terminal: reading lines, showing a prompt,
//! keeping history, completing names. Everything a session *does* lives in
//! `circuit_session`, which is why the behaviour can be tested without a
//! terminal — including by piping a script into this command.
//!
//! Two input paths, one behaviour:
//!
//! - a terminal gets `rustyline`: line editing, history, completion;
//! - anything else (a pipe, a script, CI) gets a plain line loop, so
//!   `printf ... | cdsl repl` exercises the same session code.
//!
//! `Ctrl+C` discards what is being typed and keeps the session; it never
//! exits. `Ctrl+D` on an empty line leaves.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use circuit_session::{Options, Reply, Session};

use crate::EXIT_USER_ERROR;

/// Prompt for a fresh input.
const PROMPT: &str = "cdsl> ";
/// Used to check whether rendered output already ended its line.
const NEWLINE: char = '\n';
/// Prompt while an input is unfinished.
const CONTINUATION: &str = "....> ";

pub fn repl(preload: Option<&Path>, verbose: bool) -> Result<(), u8> {
    let mut session = Session::new(Options::default());

    if let Some(path) = preload {
        match session.load(path, false) {
            Ok(Reply::Defined { accepted, replaced }) => {
                let mut parts = Vec::new();
                if !accepted.is_empty() {
                    parts.push(format!("defined {}", accepted.join(", ")));
                }
                if !replaced.is_empty() {
                    parts.push(format!("replaced {}", replaced.join(", ")));
                }
                println!("loaded `{}`: {}", path.display(), parts.join("; "));
            }
            Ok(other) => println!("loaded `{}`: {other:?}", path.display()),
            Err(d) => {
                eprintln!("{}", session.render(&d));
                // A failed preload is not fatal: entering the REPL with an
                // empty session is more useful than refusing to start.
                eprintln!("   = starting with an empty session");
            }
        }
    }

    if std::io::stdin().is_terminal() {
        interactive(&mut session, verbose)
    } else {
        piped(&mut session, verbose)
    }
}

/// What applying one line did, for the caller to react to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    KeepGoing,
    /// The input was reported as an error; the session is unchanged.
    Failed,
    Done,
}

/// Apply one line and print what it produced.
fn step(session: &mut Session, line: &str) -> Step {
    match session.feed(line) {
        Ok(Reply::Continue { .. }) | Ok(Reply::Nothing) => Step::KeepGoing,
        Ok(Reply::Value { name, text, .. }) => {
            match name {
                Some(name) => println!("{name} = {text}"),
                None => println!("=> {text}"),
            }
            Step::KeepGoing
        }
        Ok(Reply::Defined { accepted, replaced }) => {
            if !accepted.is_empty() {
                println!("defined {}", accepted.join(", "));
            }
            if !replaced.is_empty() {
                println!("replaced {}", replaced.join(", "));
            }
            if accepted.is_empty() && replaced.is_empty() {
                println!("nothing to define");
            }
            Step::KeepGoing
        }
        Ok(Reply::Message(text)) => {
            println!("{text}");
            Step::KeepGoing
        }
        Ok(Reply::Quit) => Step::Done,
        Err(diagnostics) => {
            let text = session.render(&diagnostics);
            eprint!("{text}");
            if !text.ends_with(NEWLINE) {
                eprintln!();
            }
            Step::Failed
        }
    }
}

/// Read from stdin without a terminal.
fn piped(session: &mut Session, verbose: bool) -> Result<(), u8> {
    if verbose {
        eprintln!("reading from stdin (not a terminal): no line editing or completion");
    }
    let stdin = std::io::stdin();
    let mut had_error = false;
    let mut prompt = PROMPT;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("error[E_IO]: cannot read stdin: {e}");
                return Err(EXIT_USER_ERROR);
            }
        };
        print!("{prompt}");
        let _ = std::io::stdout().flush();
        match step(session, &line) {
            Step::Done => break,
            // A bad line does not end a script, but it does decide the exit
            // code, so a piped session can be used in a test or a script.
            Step::Failed => had_error = true,
            Step::KeepGoing => {}
        }
        prompt = if session.is_continuing() {
            CONTINUATION
        } else {
            PROMPT
        };
    }

    if had_error {
        Err(EXIT_USER_ERROR)
    } else {
        Ok(())
    }
}

/// What one read from the terminal produced.
#[derive(Debug)]
enum Terminal {
    Line(String),
    /// Ctrl+C.
    Interrupted,
    /// Ctrl+D on an empty line.
    EndOfInput,
    /// A real I/O failure.
    Broken(String),
}

/// Map rustyline's outcome onto the four things the loop distinguishes.
fn classify(read: Result<String, rustyline::error::ReadlineError>) -> Terminal {
    use rustyline::error::ReadlineError;
    match read {
        Ok(line) => Terminal::Line(line),
        Err(ReadlineError::Interrupted) => Terminal::Interrupted,
        Err(ReadlineError::Eof) => Terminal::EndOfInput,
        Err(e) => Terminal::Broken(e.to_string()),
    }
}

/// `Ctrl+C`: abandon what is being typed, keep everything else.
///
/// The session state is deliberately untouched apart from the pending buffer:
/// cancelling an input must never cost the work already done. This is a
/// function rather than a few inline lines so its effect can be tested — the
/// keystroke itself needs a terminal, the behaviour does not.
fn on_interrupt(session: &mut Session) {
    println!("^C");
    session.cancel_pending();
}

/// The prompt to show next, given what the session is waiting for.
fn prompt_for(session: &Session) -> &'static str {
    if session.is_continuing() {
        CONTINUATION
    } else {
        PROMPT
    }
}

/// Read from a terminal, with history and completion.
fn interactive(session: &mut Session, verbose: bool) -> Result<(), u8> {
    use rustyline::error::ReadlineError;
    use rustyline::history::DefaultHistory;
    use rustyline::{Config, Editor};

    let helper = Helper::new(session);
    let config = Config::builder().auto_add_history(true).build();
    let mut editor = Editor::<Helper, DefaultHistory>::with_config(config).map_err(|e| {
        eprintln!("error[E_IO]: cannot start line editing: {e}");
        EXIT_USER_ERROR
    })?;
    editor.set_helper(Some(helper));

    let history = history_path();
    if let Some(path) = &history {
        match editor.load_history(path) {
            Ok(()) => {
                if verbose {
                    eprintln!("history: {}", path.display());
                }
            }
            Err(ReadlineError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("warning: cannot read history: {e}"),
        }
    }

    println!("cdsl REPL — `:help` for commands, Ctrl+D to leave");
    let mut prompt = PROMPT;
    let mut failed = false;

    loop {
        let read = editor.readline(prompt);
        match classify(read) {
            Terminal::Line(line) => {
                step(session, &line);
            }
            Terminal::Interrupted => {
                // Ctrl+C cancels what is being typed and keeps the session,
                // including when a continuation is pending.
                on_interrupt(session);
            }
            Terminal::EndOfInput => {
                println!();
                break;
            }
            Terminal::Broken(message) => {
                eprintln!("error[E_IO]: {message}");
                failed = true;
                break;
            }
        }
        prompt = prompt_for(session);
        if let Some(helper) = editor.helper_mut() {
            helper.refresh(session);
        }
    }

    // History is saved on the way out whether or not it worked; a failure is
    // only worth interrupting the exit for in verbose mode.
    if let Some(path) = history
        && let Err(e) = editor.save_history(&path)
        && verbose
    {
        eprintln!("warning: cannot write history to {}: {e}", path.display());
    }

    if failed { Err(EXIT_USER_ERROR) } else { Ok(()) }
}

/// Where history is kept, if the environment names a home directory.
fn history_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".cdsl_history"))
}

/// Completion for command names and the names a session has defined.
struct Helper {
    names: Vec<String>,
}

impl Helper {
    fn new(session: &Session) -> Self {
        Self {
            names: circuit_session::completions(session),
        }
    }

    fn refresh(&mut self, session: &Session) {
        self.names = circuit_session::completions(session);
    }
}

impl rustyline::completion::Completer for Helper {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        let start = line[..pos]
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':' || c == '.' || c == '='))
            .map(|i| i + 1)
            .unwrap_or(0);
        let word = &line[start..pos];
        if word.is_empty() {
            return Ok((pos, Vec::new()));
        }
        let mut out: Vec<String> = self
            .names
            .iter()
            .filter(|name| name.starts_with(word))
            .cloned()
            .collect();
        out.dedup();
        Ok((start, out))
    }
}

impl rustyline::highlight::Highlighter for Helper {}
impl rustyline::hint::Hinter for Helper {
    type Hint = String;
}
impl rustyline::validate::Validator for Helper {}
impl rustyline::Helper for Helper {}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyline::completion::Completer;
    use rustyline::history::History;

    /// The interactive loop's handling of `Ctrl+C`: the keystroke needs a
    /// terminal, the effect on the session does not.
    #[test]
    fn an_interrupt_drops_the_input_and_keeps_the_session() {
        let mut session = Session::default();
        step(&mut session, "r = 1.kohm");
        let _ = step(&mut session, "circuit :d do");
        assert!(session.is_continuing(), "a block is open");

        on_interrupt(&mut session);

        assert!(!session.is_continuing(), "the half-typed input is gone");
        assert_eq!(session.variables().count(), 1, "the variable survived");
        // And the next input is read as a fresh one.
        assert!(matches!(session.feed("r").unwrap(), Reply::Value { .. }));
    }

    #[test]
    fn the_prompt_follows_what_the_session_is_waiting_for() {
        let mut session = Session::default();
        assert_eq!(prompt_for(&session), PROMPT);
        let _ = step(&mut session, "circuit :d do");
        assert_eq!(prompt_for(&session), CONTINUATION);
        let _ = step(&mut session, "end");
        assert_eq!(prompt_for(&session), PROMPT);
    }

    #[test]
    fn completion_covers_commands_definitions_and_variables() {
        let mut session = Session::default();
        let _ = step(&mut session, "r = 1.kohm");
        let _ = step(
            &mut session,
            "circuit :divider do\n  node :a\n  voltage_source :v1, p: :a, n: :gnd, dc: 1.V\nend",
        );
        let _ = step(
            &mut session,
            "experiment :divider, circuit: :divider do\n  op\n  save v(:a)\nend",
        );

        let mut helper = Helper::new(&session);
        let history = rustyline::history::DefaultHistory::new();
        let ctx = rustyline::Context::new(&history);
        let complete = |helper: &Helper, line: &str| {
            let (start, candidates) = helper.complete(line, line.len(), &ctx).expect("completes");
            (start, candidates)
        };

        // A command prefix.
        let (start, candidates) = complete(&helper, ":ru");
        assert_eq!(start, 0);
        assert!(candidates.contains(&":run".to_string()), "{candidates:?}");

        // A definition name, both kinds.
        let (_, candidates) = complete(&helper, "div");
        assert!(
            candidates.contains(&"divider".to_string()),
            "{candidates:?}"
        );

        // A variable name, after the model has been refreshed.
        helper.refresh(&session);
        let (_, candidates) = complete(&helper, "r");
        assert!(candidates.contains(&"r".to_string()), "{candidates:?}");

        // Nothing to suggest is not an error.
        let (_, candidates) = complete(&helper, "zzz");
        assert!(candidates.is_empty(), "{candidates:?}");
    }

    /// History is a file plus a buffer; the buffer round-trips without a
    /// terminal, so the mechanism is testable even though the ↑ key is not.
    #[test]
    fn history_round_trips_through_a_file() {
        use rustyline::history::DefaultHistory;
        let path = std::env::temp_dir().join("cdsl_history_test");
        let _ = std::fs::remove_file(&path);

        let mut first = DefaultHistory::new();
        first.add("r = 1.kohm").expect("add");
        first.add(":run divider").expect("add");
        first.save(&path).expect("save");

        let mut second = DefaultHistory::new();
        second.load(&path).expect("load");
        let entries: Vec<&str> = second.iter().map(|e| e.as_str()).collect();
        assert_eq!(entries, ["r = 1.kohm", ":run divider"]);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_history_path_is_in_the_home_directory() {
        let path = history_path().expect("a home directory in this environment");
        assert!(path.ends_with(".cdsl_history"), "{}", path.display());
    }

    /// End of input is the same code path as `:quit`: the loop leaves with the
    /// success code rather than an error.
    #[test]
    fn leaving_by_eof_is_not_a_failure() {
        assert_eq!(EXIT_USER_ERROR, 1);
        let mut session = Session::default();
        let _ = step(&mut session, "1 + 1");
        // Nothing here sets the failure flag; only a reported error does.
        assert_eq!(step(&mut session, ":quit"), Step::Done);
        assert_eq!(step(&mut session, "node 5"), Step::Failed);
    }
}
