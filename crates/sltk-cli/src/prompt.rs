//! Questions `sltk` asks a person on a terminal. Each takes a reader and a
//! writer, so tests answer them without a terminal.

use std::io::{self, BufRead, Read, Write};

/// How many times [`confirm`], [`pick`] and [`ask`] ask before they give up:
/// the first question and three re-asks.
const ASKS: usize = 4;

/// The most bytes of one answer line that [`read_answer`] keeps. It discards
/// the rest of a longer line.
const LINE_BYTES: u64 = 4096;

/// The question [`pick`] asks after it lists the tools.
const PICK_QUESTION: &str = "Add which? Numbers or names, Enter for none: ";

/// Writes `question` and reads one line of answer. Enter, `y` and `yes` mean
/// yes; `n` and `no` mean no, in any case. Any other answer, including one
/// that is not UTF-8, asks again, up to [`ASKS`] times in all, and then counts
/// as no. End of input counts as no.
pub fn confirm(input: &mut impl BufRead, output: &mut impl Write, question: &str) -> io::Result<bool> {
    for ask in 0..ASKS {
        if ask > 0 {
            writeln!(output, "answer y or n")?;
        }
        write!(output, "{question}")?;
        output.flush()?;
        let Some(line) = read_answer(input)? else {
            writeln!(output)?;
            return Ok(false);
        };
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => {}
        }
    }
    Ok(false)
}

/// Lists the `essential` tools as always installed and each `optional` tool
/// numbered and unticked, then reads one line naming the optional tools to
/// add, by number or name in any case, separated by spaces or commas. Returns
/// the chosen names as `optional` spells them, in `optional` order.
///
/// Enter chooses none. An answer naming anything else, or not UTF-8, asks
/// again, up to [`ASKS`] times in all, and then counts as none. End of input
/// counts as none. The rejected word is echoed with control characters
/// escaped, so a typed escape sequence is shown rather than run.
pub fn pick(
    input: &mut impl BufRead, output: &mut impl Write, essential: &[&str], optional: &[&str],
) -> io::Result<Vec<String>> {
    writeln!(output, "Always installed: {}", essential.join(", "))?;
    writeln!(output, "Optional:")?;
    for (number, name) in optional.iter().enumerate() {
        writeln!(output, "  {}. [ ] {name}", number + 1)?;
    }
    for ask in 0..ASKS {
        write!(output, "{PICK_QUESTION}")?;
        output.flush()?;
        let Some(line) = read_answer(input)? else {
            writeln!(output)?;
            return Ok(Vec::new());
        };
        match chosen(&line, optional) {
            Ok(picked) => return Ok(picked),
            Err(word) if ask + 1 < ASKS => writeln!(output, "no optional tool {}", word.escape_debug())?,
            Err(_) => {}
        }
    }
    Ok(Vec::new())
}

/// Reads one line of answer, or `None` at end of input. Keeps at most
/// [`LINE_BYTES`] bytes and discards the rest of a longer line. Bytes that are
/// not UTF-8 become U+FFFD, so they make a wrong answer instead of an error.
fn read_answer(input: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut line = Vec::new();
    if Read::take(&mut *input, LINE_BYTES).read_until(b'\n', &mut line)? == 0 {
        return Ok(None);
    }
    if line.len() as u64 == LINE_BYTES && line.last() != Some(&b'\n') {
        input.skip_until(b'\n')?;
    }
    Ok(Some(String::from_utf8_lossy(&line).into_owned()))
}

/// The tools of `optional` that `line` names by number or name, in
/// `optional` order, or the first word that names none of them.
fn chosen(line: &str, optional: &[&str]) -> Result<Vec<String>, String> {
    let mut ticked = vec![false; optional.len()];
    for word in line
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|word| !word.is_empty())
    {
        let index = match word.parse::<usize>() {
            Ok(number) => number.checked_sub(1).filter(|&index| index < optional.len()),
            Err(_) => optional.iter().position(|name| name.eq_ignore_ascii_case(word)),
        };
        match index {
            Some(index) => ticked[index] = true,
            None => return Err(word.to_owned()),
        }
    }
    Ok(optional
        .iter()
        .zip(ticked)
        .filter(|(_, ticked)| *ticked)
        .map(|(name, _)| (*name).to_owned())
        .collect())
}

/// Writes `question` and reads one line of answer, trimmed. An answer that
/// `valid` rejects is written back with the reason, and the question asked
/// again, up to [`ASKS`] times in all. An answer that is not UTF-8 reaches
/// `valid` with U+FFFD in place of the bad bytes. Returns `None` when every
/// answer was rejected or input ended.
pub fn ask(
    input: &mut impl BufRead, output: &mut impl Write, question: &str, valid: impl Fn(&str) -> Result<(), String>,
) -> io::Result<Option<String>> {
    for _ in 0..ASKS {
        write!(output, "{question}")?;
        output.flush()?;
        let Some(line) = read_answer(input)? else {
            writeln!(output)?;
            return Ok(None);
        };
        let answer = line.trim();
        match valid(answer) {
            Ok(()) => return Ok(Some(answer.to_owned())),
            Err(reason) => writeln!(output, "that answer {reason}")?,
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{LINE_BYTES, PICK_QUESTION, ask, confirm, pick};

    const QUESTION: &str = "Install these? [Y/n] ";

    /// What [`pick`] lists for fab and arcl, then mire and dalil.
    const LIST: &str = "Always installed: fab, arcl\nOptional:\n  1. [ ] mire\n  2. [ ] dalil\n";

    /// The answer and what the prompt wrote for `typed`.
    fn answer(typed: &str) -> (bool, String) {
        let mut output = Vec::new();
        let yes = confirm(&mut typed.as_bytes(), &mut output, QUESTION).unwrap();
        (yes, String::from_utf8(output).unwrap())
    }

    #[test]
    fn enter_and_yes_mean_yes() {
        for typed in ["\n", "y\n", "Y\n", "yes\n", " YES \n", "Yes"] {
            assert_eq!(answer(typed), (true, QUESTION.to_owned()), "{typed:?}");
        }
    }

    #[test]
    fn n_and_no_mean_no() {
        for typed in ["n\n", "N\n", "no\n", "No\n", "NO"] {
            assert_eq!(answer(typed), (false, QUESTION.to_owned()), "{typed:?}");
        }
    }

    #[test]
    fn end_of_input_means_no() {
        assert_eq!(answer(""), (false, format!("{QUESTION}\n")));
    }

    #[test]
    fn a_bad_answer_asks_again() {
        let again = format!("{QUESTION}answer y or n\n{QUESTION}");
        assert_eq!(answer("maybe\ny\n"), (true, again.clone()));
        assert_eq!(answer("maybe\nno\n"), (false, again));
    }

    #[test]
    fn four_bad_answers_mean_no_without_reading_more() {
        let (yes, output) = answer("a\nb\nc\nd\ny\n");
        assert!(!yes);
        assert_eq!(output.matches(QUESTION).count(), 4);
        assert!(!output.ends_with("answer y or n\n"));
    }

    #[test]
    fn an_answer_that_is_not_utf8_asks_again() {
        let mut output = Vec::new();
        let yes = confirm(&mut &b"\xff\xfe\ny\n"[..], &mut output, QUESTION).unwrap();
        assert!(yes);
        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!("{QUESTION}answer y or n\n{QUESTION}")
        );
    }

    #[test]
    fn the_rest_of_an_over_long_line_is_discarded() {
        let typed = format!("{}\nno\n", "x".repeat(LINE_BYTES as usize * 2));
        let (yes, output) = answer(&typed);
        assert!(!yes);
        assert_eq!(output, format!("{QUESTION}answer y or n\n{QUESTION}"));
    }

    /// The tools chosen and what the picker wrote for `typed`.
    fn picked(typed: &str) -> (Vec<String>, String) {
        picked_bytes(typed.as_bytes())
    }

    /// The tools chosen and what the picker wrote for the bytes `typed`.
    fn picked_bytes(mut typed: &[u8]) -> (Vec<String>, String) {
        let mut output = Vec::new();
        let picked = pick(&mut typed, &mut output, &["fab", "arcl"], &["mire", "dalil"]).unwrap();
        (picked, String::from_utf8(output).unwrap())
    }

    #[test]
    fn enter_picks_nothing() {
        assert_eq!(picked("\n"), (Vec::new(), format!("{LIST}{PICK_QUESTION}")));
    }

    #[test]
    fn tools_are_picked_by_number_or_name_in_list_order() {
        assert_eq!(picked("1\n").0, ["mire"]);
        assert_eq!(picked("dalil\n").0, ["dalil"]);
        assert_eq!(picked("dalil, 1\n").0, ["mire", "dalil"]);
        assert_eq!(picked(" 2 2 dalil \n").0, ["dalil"]);
    }

    #[test]
    fn names_match_in_any_case_and_come_back_as_listed() {
        assert_eq!(picked("MIRE\n").0, ["mire"]);
        assert_eq!(picked("Dalil, mIrE\n").0, ["mire", "dalil"]);
    }

    #[test]
    fn a_wrong_pick_asks_again() {
        let again = format!("{LIST}{PICK_QUESTION}no optional tool 3\n{PICK_QUESTION}");
        assert_eq!(picked("3\nmire\n"), (vec!["mire".to_owned()], again));
        assert_eq!(
            picked("fab\n\n").1,
            format!("{LIST}{PICK_QUESTION}no optional tool fab\n{PICK_QUESTION}")
        );
        assert_eq!(picked("0\n1\n").0, ["mire"]);
    }

    #[test]
    fn a_pick_that_is_not_utf8_asks_again() {
        let again = format!("{LIST}{PICK_QUESTION}no optional tool \u{FFFD}\n{PICK_QUESTION}");
        assert_eq!(picked_bytes(b"\xff\n2\n"), (vec!["dalil".to_owned()], again));
    }

    #[test]
    fn a_wrong_pick_is_echoed_with_control_characters_escaped() {
        let (picked, output) = picked("\x1b]0;x\x07\n\n");
        assert!(picked.is_empty());
        assert!(!output.contains(['\x1b', '\x07']), "{output:?}");
        assert!(output.contains("no optional tool \\u{1b}]0;x\\u{7}\n"), "{output:?}");
    }

    #[test]
    fn end_of_input_picks_nothing() {
        assert_eq!(picked(""), (Vec::new(), format!("{LIST}{PICK_QUESTION}\n")));
    }

    #[test]
    fn four_wrong_picks_mean_nothing_without_reading_more() {
        let (picked, output) = picked("a\nb\nc\nd\n1\n");
        assert!(picked.is_empty());
        assert_eq!(output.matches(PICK_QUESTION).count(), 4);
        assert!(output.ends_with(PICK_QUESTION), "{output}");
    }

    /// What [`ask`] returns and writes for `typed`, accepting any answer that
    /// is not empty.
    fn asked(typed: &str) -> (Option<String>, String) {
        let mut output = Vec::new();
        let not_empty = |a: &str| if a.is_empty() { Err("is empty".to_owned()) } else { Ok(()) };
        let answer = ask(&mut typed.as_bytes(), &mut output, "Owner? ", not_empty).unwrap();
        (answer, String::from_utf8(output).unwrap())
    }

    #[test]
    fn ask_returns_the_trimmed_answer() {
        assert_eq!(asked("  octo \n"), (Some("octo".to_owned()), "Owner? ".to_owned()));
    }

    #[test]
    fn ask_repeats_a_rejected_answer_with_the_reason() {
        let again = "Owner? that answer is empty\nOwner? ".to_owned();
        assert_eq!(asked("\nocto\n"), (Some("octo".to_owned()), again));
    }

    #[test]
    fn ask_gives_up_after_four_rejections_or_at_end_of_input() {
        let (answer, output) = asked("\n\n\n\nocto\n");
        assert_eq!(answer, None);
        assert_eq!(output.matches("Owner? ").count(), 4);
        assert_eq!(asked(""), (None, "Owner? \n".to_owned()));
    }

    #[test]
    fn ask_reads_an_answer_that_is_not_utf8_as_a_wrong_one() {
        let mut output = Vec::new();
        let ascii = |a: &str| if a.is_ascii() { Ok(()) } else { Err("is not ASCII".to_owned()) };
        let answer = ask(&mut &b"oct\xff\nocto\n"[..], &mut output, "Owner? ", ascii).unwrap();
        assert_eq!(answer, Some("octo".to_owned()));
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "Owner? that answer is not ASCII\nOwner? "
        );
    }
}
