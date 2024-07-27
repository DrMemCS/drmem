use std::{iter::Fuse, str::Chars};

// Defines a finite-state machine that traverses the pattern and tries
// to match a name.

struct Fsm<'a> {
    stack: Vec<(Fuse<Chars<'a>>, Fuse<Chars<'a>>)>,

    it_pat: Fuse<Chars<'a>>,
    it_name: Fuse<Chars<'a>>,

    ch_pat: Option<char>,
    ch_name: Option<char>,
}

impl Fsm<'_> {
    pub fn new<'a>(pat: &'a str, name: &'a str) -> Fsm<'a> {
        Fsm {
            stack: vec![],
            it_pat: pat.chars().fuse(),
            it_name: name.chars().fuse(),
            ch_pat: None,
            ch_name: None,
        }
    }

    fn next_pat_char(&mut self) -> bool {
        self.ch_pat = self.it_pat.next();
        self.ch_pat.is_some()
    }

    fn next_name_char(&mut self) {
        self.ch_name = self.it_name.next();
    }

    fn save(&mut self) {
        self.stack.push((self.it_pat.clone(), self.it_name.clone()))
    }

    fn backtrace(&mut self) -> Option<bool> {
        if let Some((i_p, i_n)) = self.stack.pop() {
            self.it_pat = i_p;
            self.it_name = i_n;
            self.next_pat_char();
            self.next_name_char();
            None
        } else {
            Some(false)
        }
    }

    fn step(&mut self) -> Option<bool> {
        match (self.ch_pat, self.ch_name) {
            (None, None) | (Some('*'), None) => Some(true),

            (None, _) | (_, None) => self.backtrace(),

            (Some('?'), Some(_)) => {
                self.next_pat_char();
                self.next_name_char();
                None
            }

            (Some('*'), Some(_)) => {
                // This loop advances the pattern iterator to the
                // next literal character. If a '?' character is
                // found, it also advances the name iterator.

                while self.ch_pat == Some('?') || self.ch_pat == Some('*') {
                    // If there are no remaining pattern characters,
                    // then all trailing characters were wildcards and
                    // the pattern matches.

                    if !self.next_pat_char() {
                        return Some(true);
                    }

                    // If the next character is a '?', consume, if
                    // possible, the next name character. Otherwise
                    // look for another solution.

                    if let Some('?') = self.ch_pat {
                        if self.ch_name.is_none() {
                            return self.backtrace();
                        }
                        self.next_name_char();
                    }
                }

                while self.ch_name.is_some() {
                    if self.ch_name == self.ch_pat {
                        self.save()
                    }
                    self.next_name_char()
                }

                self.backtrace()
            }

            (Some(a), Some(b)) => {
                if a != b {
                    self.backtrace()
                } else {
                    self.next_pat_char();
                    self.next_name_char();
                    None
                }
            }
        }
    }

    pub fn process(&mut self) -> bool {
        self.next_pat_char();
        self.next_name_char();

        loop {
            if let Some(result) = self.step() {
                return result;
            }
        }
    }
}

pub struct Pattern {
    data: String,
}

impl Pattern {
    pub fn create(pat: &str) -> Pattern {
        Pattern {
            data: String::from(pat),
        }
    }

    pub fn matches(&self, name: &str) -> bool {
        let mut fsm = Fsm::new(&self.data, name);

        fsm.process()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_match() {
        // Empty input.

        assert_eq!(Pattern::create("").matches(""), true);

        // Degenerate cases.

        assert_eq!(Pattern::create("a").matches(""), false);
        assert_eq!(Pattern::create("").matches("a"), false);

        // Literal macthing.

        assert_eq!(Pattern::create("a").matches("a"), true);
        assert_eq!(Pattern::create("a").matches("b"), false);
        assert_eq!(Pattern::create("ab").matches("ab"), true);
        assert_eq!(Pattern::create("abc").matches("abc"), true);

        // Wildcard matching.

        assert_eq!(Pattern::create("?").matches("a"), true);
        assert_eq!(Pattern::create("?").matches("z"), true);
        assert_eq!(Pattern::create("a?").matches("ab"), true);
        assert_eq!(Pattern::create("a?").matches("a"), false);
        assert_eq!(Pattern::create("?bc").matches("abc"), true);
        assert_eq!(Pattern::create("a?c").matches("abc"), true);
        assert_eq!(Pattern::create("ab?").matches("abc"), true);
        assert_eq!(Pattern::create("??c").matches("abc"), true);
        assert_eq!(Pattern::create("?b?").matches("abc"), true);
        assert_eq!(Pattern::create("a??").matches("abc"), true);
        assert_eq!(Pattern::create("???").matches("abc"), true);
        assert_eq!(Pattern::create("a?c?e").matches("abcde"), true);

        // Multi-wildcard matching.

        assert_eq!(Pattern::create("*").matches(""), true);
        assert_eq!(Pattern::create("a*").matches("a"), true);
        assert_eq!(Pattern::create("a*").matches("aa"), true);
        assert_eq!(Pattern::create("a*b").matches("ab"), true);
        assert_eq!(Pattern::create("a*b").matches("azb"), true);
        assert_eq!(Pattern::create("a*b").matches("azzzzzzzzb"), true);
        assert_eq!(Pattern::create("a*bc").matches("azbc"), true);

        // Interspersed wildcards.

        assert_eq!(Pattern::create("a***bc").matches("azbc"), true);
        assert_eq!(Pattern::create("a?*cd").matches("azbcd"), true);
        assert_eq!(Pattern::create("a*?cd").matches("azbcd"), true);
        assert_eq!(Pattern::create("a*?*cd").matches("azbcd"), true);
        assert_eq!(Pattern::create("a*c*e").matches("abcde"), true);
        assert_eq!(Pattern::create("a*?de").matches("abcde"), true);

        // Devious names requiring backtracking.

        assert_eq!(Pattern::create("a*bc").matches("azbcbc"), true);
        assert_eq!(Pattern::create("a*bc").matches("azbdbc"), true);
        assert_eq!(Pattern::create("a*bc*").matches("azbcbc"), true);
        assert_eq!(Pattern::create("a*bc*").matches("azbcbd"), true);
        assert_eq!(Pattern::create("a*bc").matches("azbcd"), false);
    }
}
