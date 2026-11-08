//! Questions `sltk` asks a person on a terminal. Each takes a reader and a
//! writer, so tests answer them without a terminal.

use std::io::{self, BufRead, Write};

/// How many times [`confirm`] and [`pick`] ask before they treat the answers
/// as no: the first question and three re-asks.
const ASKS: usize = 4;

/// The question [`pick`] asks after it lists the tools.
const PICK_QUESTION: &str = "Add which? Numbers or names, Enter for none: ";

/// Writes `question` and reads one line of answer. Enter, `y` and `yes` mean
/// yes; `n` and `no` mean no, in any case. Any other answer asks again, up to
/// [`ASKS`] times in all, and then counts as no. End of input counts as no.
pub fn confirm(input: &mut impl BufRead, output: &mut impl Write, question: &str) -> io::Result<bool> {
    for ask in 0..ASKS {
        if ask > 0 {
            writeln!(output, "answer y or n")?;
        }
        write!(output, "{question}")?;
        output.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            writeln!(output)?;
            return Ok(false);
        }
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
/// add, by number or name, separated by spaces or commas. Returns the chosen
/// names in `optional` order.
///
/// Enter chooses none. An answer naming anything else asks again, up to
/// [`ASKS`] times in all, and then counts as none. End of input counts as
/// none.
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
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            writeln!(output)?;
            return Ok(Vec::new());
        }
        match chosen(&line, optional) {
            Ok(picked) => return Ok(picked),
            Err(word) if ask + 1 < ASKS => writeln!(output, "no optional tool {word}")?,
            Err(_) => {}
        }
    }
    Ok(Vec::new())
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
            Err(_) => optional.iter().position(|name| *name == word),
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

#[cfg(test)]
mod tests {
    use super::{PICK_QUESTION, confirm, pick};

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

    /// The tools chosen and what the picker wrote for `typed`.
    fn picked(typed: &str) -> (Vec<String>, String) {
        let mut output = Vec::new();
        let picked = pick(&mut typed.as_bytes(), &mut output, &["fab", "arcl"], &["mire", "dalil"]).unwrap();
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
}
