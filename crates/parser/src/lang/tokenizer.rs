use std::{cell::RefCell, iter::Peekable, str::Chars};
use crate::lang::ast::*;

#[derive(Debug, Clone, Copy)]
pub enum TokenizerErr {
    UnterminatedStringLiteral,
    UnexpectedToken,
    EmptyElementIdentifier,
    InvalidElementIdentifier
}

pub type TokenResult = Result<Token, TokenizerErr>;

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
                self.pending.replace(Some(Token {
                    loc,
                    con: TokenContent::Identifier(word),
                }));

                return None;
            }
        };

        self.pending.replace(Some(Token {
            loc,
            con: TokenContent::Identifier(word),
        }));

        None
    }

    fn lex_identifier(&mut self) -> TokenResult {
        let mut word = String::new();
        let mut loc = TokenLoc {
            starts_at: self.current_idx,
            len: 0,
        };

        if let Some(pending) = &*self.pending.borrow_mut() {
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

    fn lex_element_identifier(&mut self) -> TokenResult {
        if let Some(&c) = self.itr.peek() {
            let mut loc = TokenLoc {
                starts_at: self.current_idx,
                len: 0,
            };
            if c != '@' {
                return Err(TokenizerErr::InvalidElementIdentifier);
            }

            let mut identifier = String::new();

            identifier.push(c);
            self.advance();
            loc.len += 1;

            while let Some(&c) = self.itr.peek() {
                if c.is_whitespace() {
                    break;
                } else if c.is_alphabetic() {
                    identifier.push(c);
                } else {
                    break;
                }

                self.advance();
                loc.len += 1;
            }

            if identifier.is_empty() {
                return Err(TokenizerErr::EmptyElementIdentifier);
            }

            return Ok(Token {
                loc,
                con: TokenContent::Element(identifier),
            });
        } else {
            return Err(TokenizerErr::InvalidElementIdentifier);
        }
    }

    fn advance(&mut self) {
       self.next_char();
    }

    fn next_char(&mut self) {
        self.itr.next();

        self.current_idx += 1;

        if self.current_idx == MAX_IDX_VALUE {
            self.full_idx_count += 1;
            self.current_idx = 0;
        }
    }

    pub fn next(&mut self) -> Option<TokenResult> {
        while let Some(&c) = self.itr.peek() {
            if c.is_whitespace() {
                self.advance();
                continue;
            } else if c.is_digit(10) {
                return Some(self.lex_number_literal());
            } else if c.is_alphabetic() {
                return Some(self.lex_alphabetical_chars());
            } else if c == '"' {
                return Some(self.lex_string_literal());
            } else if c == '@' {
                return Some(self.lex_element_identifier());
            }
        };
        None
    }
}

#[cfg(test)]
mod test {
    use super::*;

    struct QueryTest<'a> {
        name: &'a str,
        expected: Vec<Token>,
        query: &'a str,
    }

    impl<'a> QueryTest<'a> {
        pub fn new(name: &'a str, expected: Vec<Token>, query: &'a str) -> Self {
            Self {
                name,
                expected,
                query,
            }
        }

        pub fn run(&self) -> QueryTestResult {
            let mut tokenizer = Tokenizer::new(self.query);
            let mut i: usize = 0;
            while let Some(token) = tokenizer.next() {
                match token {
                    Ok(token) => {
                        if token != self.expected[i] {
                            println!("{}: Failed with unexpected token\n- Expected:\n{:?}\n- Result:\n{:?}", self.name, self.expected[i], token);
                            return Err(QueryTestErr::UnexpectedToken);
                        }
                    },
                    Err(err) => {
                        println!("{}: Failed with tokenizer error\n- Error:\n{:?}", self.name, err);
                        return Err(QueryTestErr::TokenizerError);
                    }
                }
                i += 1;
            };
            println!("{}: Passed", self.name);
            Ok(())
        }
    }

    enum QueryTestErr {
        TokenizerError,
        UnexpectedToken,
    }

    type QueryTestResult = Result<(), QueryTestErr>;

    struct QueryTester<'a> {
        tests: Vec<QueryTest<'a>>,
    }

    impl<'a> QueryTester<'a> {
        pub fn new() -> Self {
            Self {
                tests: Vec::new(),
            }
        }

        pub fn add_test(&mut self, test: QueryTest<'a>) {
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
        assert!(QueryTest::new("numeric literals",
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
        assert!(QueryTest::new("multiple tokens",
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
        assert!(QueryTest::new("string literal",
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
    fn lex_queries() {
        let mut tester = QueryTester::new();
        tester.add_test(QueryTest::new("create query",
            vec![Token {
                loc: TokenLoc {
                    starts_at: 0,
                    len: 5,
                },
                con: TokenContent::Element("@root".to_string()),
            },
            Token {
                loc: TokenLoc {
                    starts_at: 6,
                    len: 6,
                },
                con: TokenContent::Insert,
            },
            Token {
                loc: TokenLoc {
                    starts_at: 13,
                    len: 3,
                },
                con: TokenContent::New,
            },
            Token {
                loc: TokenLoc {
                    starts_at: 17,
                    len: 7,
                },
                con: TokenContent::Identifier("Element".to_string()),
            }],
            "@root insert new Element",
        ));

        tester.run_all();
    }
}
