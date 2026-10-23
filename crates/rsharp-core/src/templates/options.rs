//! The enable conditions in dotnet template parameters. Unknown grammar remains enabled, as Passenger does.

use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;

pub fn is_enabled(condition: Option<&str>, values: &BTreeMap<String, String>) -> bool {
    static TOKENS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#""[^"]*"|'[^']*'|==|!=|&&|\|\||[!()]|[^\s!=&|()"']+"#).unwrap());
    let Some(condition) = condition.filter(|s| !s.trim().is_empty()) else { return true };
    let mut p = Parser { tokens: TOKENS.find_iter(condition).map(|m| m.as_str()).collect(), at: 0, values };
    match p.or() {
        Ok(value) if p.at == p.tokens.len() => value,
        _ => true,
    }
}

struct Parser<'a> {
    tokens: Vec<&'a str>,
    at: usize,
    values: &'a BTreeMap<String, String>,
}
impl Parser<'_> {
    fn next(&mut self, token: &str) -> bool {
        if self.tokens.get(self.at) == Some(&token) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn or(&mut self) -> Result<bool, ()> {
        let mut value = self.and()?;
        while self.next("||") {
            value |= self.and()?;
        }
        Ok(value)
    }
    fn and(&mut self) -> Result<bool, ()> {
        let mut value = self.not()?;
        while self.next("&&") {
            value &= self.not()?;
        }
        Ok(value)
    }
    fn not(&mut self) -> Result<bool, ()> {
        if self.next("!") {
            return Ok(!self.not()?);
        }
        if self.next("(") {
            let value = self.or()?;
            return if self.next(")") { Ok(value) } else { Err(()) };
        }
        let left = self.operand()?;
        if self.next("==") {
            Ok(left.eq_ignore_ascii_case(&self.operand()?))
        } else if self.next("!=") {
            Ok(!left.eq_ignore_ascii_case(&self.operand()?))
        } else {
            Ok(left.eq_ignore_ascii_case("true"))
        }
    }
    fn operand(&mut self) -> Result<String, ()> {
        let token = *self.tokens.get(self.at).ok_or(())?;
        if matches!(token, "==" | "!=" | "&&" | "||" | "!" | "(" | ")") {
            return Err(());
        }
        self.at += 1;
        Ok(if token.starts_with(['"', '\'']) {
            token[1..token.len() - 1].into()
        } else {
            self.values.get(token).cloned().unwrap_or_else(|| token.into())
        })
    }
}
