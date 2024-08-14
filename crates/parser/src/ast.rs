#[derive(PartialEq, Clone, Debug)]
pub enum ItemContent {
    ComponentDeclaration,
    Query,
}

#[derive(PartialEq, Clone, Copy, Debug)]
pub struct ItemLoc {
    pub starts_at: u32,
    pub len: u32,
}

#[derive(PartialEq, Clone, Debug)]
pub struct Item {
    pub con: ItemContent,
    pub loc: ItemLoc,
}

#[derive(PartialEq, Clone, Copy, Debug)]
pub struct TokenLoc {
    pub starts_at: u32,
    pub len: u32,
}

#[derive(PartialEq, Clone, Debug)]
pub enum TokenContent {
    Element(String),
    Identifier(String),
    NumberLiteral(String),
    StringLiteral(String),
    BraceLeft,
    BraceRight,
    Const,
    Else,
    Enum,
    False,
    FnKeyword,
    FromKeyword,
    For,
    Global,
    If,
    Impl,
    Import,
    Int,
    Let,
    Match,
    Null,
    Number,
    Pub,
    StringKeyword,
    Struct,
    Trait,
    True,
    Undefined,
    With,
}

impl TokenContent {
    pub fn from_str(word: &str) -> Option<Self> {
        match word {
            "const" => Some(Self::Const),
            "else" => Some(Self::Else),
            "enum" => Some(Self::Enum),
            "false" => Some(Self::False),
            "fn" => Some(Self::FnKeyword),
            "from" => Some(Self::FromKeyword),
            "for" => Some(Self::For),
            "global" => Some(Self::Global),
            "if" => Some(Self::If),
            "impl" => Some(Self::Impl),
            "import" => Some(Self::Import),
            "int" => Some(Self::Int),
            "let" => Some(Self::Let),
            "match" => Some(Self::Match),
            "null" => Some(Self::Null),
            "number" => Some(Self::Number),
            "pub" => Some(Self::Pub),
            "string" => Some(Self::StringKeyword),
            "struct" => Some(Self::Struct),
            "trait" => Some(Self::Trait),
            "true" => Some(Self::True),
            "undefined" => Some(Self::Undefined),
            "with" => Some(Self::With),
            _ => None,
        }
    }

    pub fn from_char(c: char) -> Option<Self> {
        match c {
            '{' => Some(Self::BraceLeft),
            '}' => Some(Self::BraceRight),
            _ => None
        }
    }
}
#[derive(PartialEq, Clone, Debug)]
pub struct Token {
    pub loc: TokenLoc,
    pub con: TokenContent,
}
