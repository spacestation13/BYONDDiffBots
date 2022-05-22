use super::atoms::*;
use nom::{bytes::complete::tag, character::complete::alpha1, sequence::separated_pair, IResult};

#[derive(Debug, PartialEq, Eq)]
pub enum Key {
    Version,
    State,
    Dirs,
    Frames,
    Delay,
    Loop,
    Rewind,
    Movement,
    Hotspot,
    Unk(String),
}

pub fn key(input: &str) -> IResult<&str, Key> {
    let (tail, key) = alpha1(input)?;
    Ok((
        tail,
        match key {
            "version" => Key::Version,
            "state" => Key::State,
            "dirs" => Key::Dirs,
            "frames" => Key::Frames,
            "delay" => Key::Delay,
            "loop" => Key::Loop,
            "rewind" => Key::Rewind,
            "movement" => Key::Movement,
            "hotspot" => Key::Hotspot,
            _ => Key::Unk(key.to_string()),
        },
    ))
}

#[derive(Debug, PartialEq)]
pub enum KeyValue {
    Version(f32),
    State(String),
    Dirs(u32),
    Frames(u32),
    Delay(Vec<f32>),
    Loop(bool),
    Rewind(bool),
    Movement(bool),
    Hotspot(Vec<f32>),
    Unk(Key, Atom),
}

macro_rules! kv {
    ($in:pat => $out:expr, $value:expr, $tail:expr) => {{
        match $value {
            $in => Ok(($tail, $out)),
            _ => Err(nom::Err::Failure(nom::error::Error::new(
                $tail,
                nom::error::ErrorKind::MapRes,
            ))),
        }
    }};
}

pub fn key_value(input: &str) -> IResult<&str, KeyValue> {
    let (tail, (key, value)) = separated_pair(key, tag(" = "), atom)(input)?;

    match key {
        Key::Version => {
            kv!(Atom::Float(x) => KeyValue::Version(x), value, tail)
        }
        Key::State => {
            kv!(Atom::String(x) => KeyValue::State(x), value, tail)
        }
        Key::Dirs => {
            kv!(Atom::Int(x) => KeyValue::Dirs(x), value, tail)
        }
        Key::Frames => {
            kv!(Atom::Int(x) => KeyValue::Frames(x), value, tail)
        }
        Key::Delay => {
            kv!(Atom::List(x) => KeyValue::Delay(x), value, tail)
        }
        Key::Loop => {
            kv!(Atom::Int(x) => KeyValue::Loop(x > 0), value, tail)
        }
        Key::Rewind => {
            kv!(Atom::Int(x) => KeyValue::Rewind(x > 0), value, tail)
        }
        Key::Movement => {
            kv!(Atom::Int(x) => KeyValue::Movement(x > 0), value, tail)
        }
        Key::Hotspot => {
            kv!(Atom::List(x) => KeyValue::Hotspot(x), value, tail)
        }
        Key::Unk(_) => Ok((tail, KeyValue::Unk(key, value))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version() {
        assert_eq!(
            key_value(r#"version = 4.0"#),
            Ok(("", (KeyValue::Version(4.0))))
        );
    }

    #[test]
    fn state() {
        assert_eq!(
            key_value(r#"state = "meow""#),
            Ok(("", KeyValue::State("meow".to_string())))
        );
    }

    #[test]
    fn dirs() {
        assert_eq!(key_value(r#"dirs = 4"#), Ok(("", (KeyValue::Dirs(4)))));
    }

    #[test]
    fn frames() {
        assert_eq!(key_value(r#"frames = 2"#), Ok(("", KeyValue::Frames(2))));
    }

    #[test]
    fn delay() {
        assert_eq!(
            key_value(r#"delay = 1,2,3"#),
            Ok(("", KeyValue::Delay(Vec::from([1.0, 2.0, 3.0]))))
        );
    }

    #[test]
    fn lööp() {
        assert_eq!(key_value(r#"loop = 1"#), Ok(("", KeyValue::Loop(true))));
    }

    #[test]
    fn rewind() {
        assert_eq!(key_value(r#"rewind = 1"#), Ok(("", KeyValue::Rewind(true))));
    }

    #[test]
    fn movement() {
        assert_eq!(
            key_value(r#"movement = 1"#),
            Ok(("", KeyValue::Movement(true)))
        );
    }

    #[test]
    fn hotspot() {
        assert_eq!(
            key_value(r#"hotspot = 13,12,1"#),
            Ok(("", KeyValue::Hotspot(Vec::from([13.0, 12.0, 1.0]))))
        );
    }

    #[test]
    fn test_evil_delay() {
        let evil_delay = r#"delay = 1,2,5.4,3"#;
        assert_eq!(
            key_value(evil_delay),
            Ok(("", (KeyValue::Delay(Vec::from([1.0, 2.0, 5.4, 3.0])))))
        );
    }
}
