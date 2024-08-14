use std::{cell::RefCell, iter::Peekable, str::Chars};
use crate::ast::*;

#[derive(Debug, Clone, Copy)]
pub enum TokenizerErr {
    UnterminatedStringLiteral,
    UnexpectedToken,
    EmptyElementIdentifier,
    InvalidElementIdentifier
}

pub type TokenResult = Result<Token, TokenizerErr>;
type TokenizationResult = Result<(), TokenizerErr>;

pub struct Tokenizer<'a>
{
    itr: Peekable<Chars<'a>>,
    pending: RefCell<Option<Token>>,
    current_idx: u32,
    full_idx_count: u32,
}

const MAX_IDX_VALUE: u32 = u32::MAX;

impl<'a> Tokenizer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            itr: input.chars().peekable(),
            pending: RefCell::new(None),
            current_idx: 0,
            full_idx_count: 0,
        }
    }

    fn lex_number_literal(&mut self) -> TokenResult {
        let mut loc = TokenLoc {
            starts_at: self.current_idx,
            len: 0,
        };

        let mut literal = String::new();
        let mut len = 0;
        while let Some(&c) = self.itr.peek() {
            self.advance();
            if c.is_digit(10) {
                literal.push(c);
                len += 1;
            } else {
                break;
            }
        };

        loc.len = len;
        Ok(Token {
            loc,
            con: TokenContent::NumberLiteral(literal),
        })
    }

    fn lex_string_literal(&mut self) -> TokenResult {
        let mut literal = String::new();
        let mut loc = TokenLoc {
            starts_at: self.current_idx,
            len: 0,
        };
        let mut quotation_mark_count = 0;
        while let Some(&c) = self.itr.peek() {
            literal.push(c);
            self.advance();
            loc.len += 1;
            if c == '\"' {
                if quotation_mark_count >= 2 {
                    break;
                }
                quotation_mark_count += 1;
            }
        };

        if quotation_mark_count < 2 {
            Err(TokenizerErr::UnterminatedStringLiteral)
        } else {
            Ok(Token {
                loc,
                con: TokenContent::StringLiteral(literal),
            })
        }
    }

    fn lex_reserved(&mut self) -> Option<TokenResult> {
        let mut word = String::new();
        let mut loc = TokenLoc {
            starts_at: self.current_idx,
            len: 0,
        };
        while let Some(&c) = self.itr.peek() {
            if c.is_alphabetic() {
                self.advance();
                loc.len += 1;
                word.push(c);
                if let Some(con) = TokenContent::from_str(word.as_str()) {
                    return Some(Ok(Token {
                        loc,
                        con
                    }));
                };
            } else {
                assert!(self.pending.replace(Some(Token {
                    loc,
                    con: TokenContent::Identifier(word),
                })).is_none());

                return None;
            }
        };

        assert!(self.pending.replace(Some(Token {
            loc,
            con: TokenContent::Identifier(word),
        })).is_none());

        None
    }

    fn lex_identifier(&mut self) -> TokenResult {
        let mut word = String::new();
        let mut loc = TokenLoc {
            starts_at: self.current_idx,
            len: 0,
        };

        if let Some(pending) = self.pending.take() {
            loc = pending.loc;

            match &pending.con {
                TokenContent::Identifier(s) => {
                    word = s.to_string();
                },
                _ => {
                    return Err(TokenizerErr::UnexpectedToken);
                },
            }
        };

        while let Some(&c) = self.itr.peek() {
            if c.is_whitespace() {
                break;
            } else if c == ';' {
                break;
            }

            word.push(c);
            loc.len += 1;
            self.itr.next();
        };

        Ok(Token {
            loc,
            con: TokenContent::Identifier(word),
        })
    }

    fn lex_alphabetical_chars(&mut self) -> TokenResult {
        if let Some(token) = self.lex_reserved() {
            return token;
        } else {
            return self.lex_identifier();
        }
    }

    fn advance(&mut self) {
       self.consume_char();
    }

    fn consume_char(&mut self) {
        self.itr.next();

        self.current_idx += 1;

        if self.current_idx == MAX_IDX_VALUE {
            self.full_idx_count += 1;
            self.current_idx = 0;
        }
    }

    fn set_pending(&mut self, token: Token) -> TokenizationResult {
        assert!(self.pending.replace(Some(token)).is_none(),
            "there is an another pending token that has not been returned yet");

        Ok(())
    }

    fn set_pending_or_err(&mut self, res: TokenResult) -> TokenizationResult {
        match res {
            Ok(token) => {
                self.set_pending(token)
            }
            Err(err) => Err(err)
        }
    }

    #[allow(unused)]
    fn make_token(&mut self, con: TokenContent, len: u32) -> Token {
        Token {
            con,
            loc: TokenLoc {
                starts_at: self.current_idx,
                len
            },
        }
    }

    fn make_token_from_char(&mut self, c: char) -> Token {
        let con = TokenContent::from_char(c).expect("unexpected character matches");
        let loc = TokenLoc {
            starts_at: self.current_idx,
            len: 1
        };

        self.advance();

        Token {
            con,
            loc
        }
    }

    fn dispatch_char(&mut self, c: char) -> TokenizationResult {
        match c {
            'a'..='z' | 'A'..='Z' => {
                let res = self.lex_alphabetical_chars();
                self.set_pending_or_err(res)
            }
            '_' | '$' => {
                let res = self.lex_identifier();
                self.set_pending_or_err(res)
            }
            '0'..='9' => {
                let res = self.lex_number_literal();
                self.set_pending_or_err(res)
            }
            '"' => {
                let res = self.lex_string_literal();
                self.set_pending_or_err(res)
            }
            '{' => {
                let token = self.make_token_from_char('{');
                self.set_pending(token)
            }
            '}' => {
                let token = self.make_token_from_char('}');
                self.set_pending(token)
            }
            _ => {
                Err(TokenizerErr::UnexpectedToken)
            }
        }
    }

    pub fn next(&mut self) -> Option<TokenResult> {
        while let Some(&c) = self.itr.peek() {
            if c.is_whitespace() {
                self.advance();
                continue;
            }

            match self.dispatch_char(c) {
                Ok(..) => {
                    if let Some(token) = self.pending.take() {
                        return Some(Ok(token));
                    } else {
                        panic!("no pending token");
                    };
                }
                Err(err) => return Some(Err(err)),
            }
        };
        None
    }
}

#[cfg(test)]
mod test {
    use super::*;

    struct TokenizerTest<'a> {
        name: &'a str,
        expected: Vec<Token>,
        query: &'a str,
    }

    impl<'a> TokenizerTest<'a> {
        pub fn new(name: &'a str, expected: Vec<Token>, query: &'a str) -> Self {
            Self {
                name,
                expected,
                query,
            }
        }

        pub fn run(&self) -> TokenizerTestResult {
            let mut tokenizer = Tokenizer::new(self.query);
            let mut i: usize = 0;
            while let Some(token) = tokenizer.next() {
                match token {
                    Ok(token) => {
                        if token != self.expected[i] {
                            println!("{}: Failed with unexpected token\n- Expected:\n{:?}\n- Result:\n{:?}", self.name, self.expected[i], token);
                            return Err(TokenizerTestErr::UnexpectedToken);
                        }
                    },
                    Err(err) => {
                        println!("{}: Failed with tokenizer error\n- Error:\n{:?}", self.name, err);
                        return Err(TokenizerTestErr::TokenizerError);
                    }
                }
                i += 1;
            };
            println!("{}: Passed", self.name);
            Ok(())
        }
    }

    enum TokenizerTestErr {
        TokenizerError,
        UnexpectedToken,
    }

    type TokenizerTestResult = Result<(), TokenizerTestErr>;

    struct TokenizerTester<'a> {
        tests: Vec<TokenizerTest<'a>>,
    }

    impl<'a> TokenizerTester<'a> {
        pub fn new() -> Self {
            Self {
                tests: Vec::new(),
            }
        }

        pub fn add_test(&mut self, test: TokenizerTest<'a>) {
            self.tests.push(test);
        }

        pub fn run_all(&mut self) {
            for test in &self.tests {
                assert!(test.run().is_ok());
            }
        }
    }

    #[test]
    fn decimal_digits() {
        assert!(TokenizerTest::new("numeric literals",
                vec![Token {
                    loc: TokenLoc {
                        starts_at: 0,
                        len: 2,
                    },
                    con: TokenContent::NumberLiteral("91".to_string()),
                }],
                "91",
            ).run().is_ok());
    }

    #[test]
    fn multiple_tokens() {
        assert!(TokenizerTest::new("multiple tokens",
                vec![Token {
                    loc: TokenLoc {
                        starts_at: 0,
                        len: 1,
                    },
                    con: TokenContent::Identifier("x".to_string()),
                },
                Token {
                    loc: TokenLoc {
                        starts_at: 2,
                        len: 2,
                    },
                    con: TokenContent::NumberLiteral("91".to_string()),
                }],
                "x 91",
        ).run().is_ok());
    }

    #[test]
    fn string_literal() {
        assert!(TokenizerTest::new("string literal",
                vec![Token {
                    loc: TokenLoc {
                        starts_at: 0,
                        len: 14,
                    },
                    con: TokenContent::StringLiteral("\"hello, world\"".to_string()),
                }],
                "\"hello, world\"",
        ).run().is_ok());
    }

    #[test]
    fn lex() {
        let mut tester = TokenizerTester::new();

        tester.add_test(TokenizerTest::new("main function",
            vec![Token {
                loc: TokenLoc {
                    starts_at: 0,
                    len: 2,
                },
                con: TokenContent::FnKeyword,
            },
            Token {
                loc: TokenLoc {
                    starts_at: 3,
                    len: 4,
                },
                con: TokenContent::Identifier("main".to_string()),
            },
            Token {
                loc: TokenLoc {
                    starts_at: 8,
                    len: 1,
                },
                con: TokenContent::BraceLeft,
            },
            Token {
                loc: TokenLoc {
                    starts_at: 9,
                    len: 1,
                },
                con: TokenContent::BraceRight,
            }],
            "fn main {}",
        ));

        tester.run_all();
    }
}
