use super::atoms::*;
use nom::{bytes::complete::tag, character::complete::alpha1, sequence::separated_pair, IResult};

pub fn key(input: &str) -> IResult<&str, &str> {
    alpha1(input)
}

pub fn key_value(input: &str) -> IResult<&str, (&str, Atom)> {
    separated_pair(key, tag(" = "), atom)(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kv() {
        let kv = r#"version = 4.0"#;
        assert_eq!(key_value(kv), Ok(("", ("version", Atom::Float(4.0)))));
    }

    #[test]
    fn test_evil_delay() {
        let evil_delay = r#"delay = 1,2,5.4,3"#;
        assert_eq!(
            key_value(evil_delay),
            Ok(("", ("delay", Atom::List(Vec::from([1.0, 2.0, 5.4, 3.0])))))
        );
    }
}
