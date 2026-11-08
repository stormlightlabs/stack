//! Questions `sltk` asks a person on a terminal. Each takes a reader and a
//! writer, so tests answer them without a terminal.

use std::io::{self, BufRead, Write};

/// How many times [`confirm`] asks before it treats the answers as no: the
/// first question and three re-asks.
const ASKS: usize = 4;

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

#[cfg(test)]
mod tests {
    use super::confirm;

    const QUESTION: &str = "Install these? [Y/n] ";

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
}
