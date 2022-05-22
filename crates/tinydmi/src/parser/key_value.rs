use super::values::*;
use nom::{
    bytes::complete::tag, character::complete::alpha1, combinator::map_opt,
    sequence::separated_pair, IResult,
};

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

#[derive(Debug, PartialEq, Eq)]
pub enum Dirs {
    One,
    Four,
    Eight,
}

impl TryFrom<u32> for Dirs {
    type Error = std::io::Error;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Dirs::One),
            4 => Ok(Dirs::Four),
            8 => Ok(Dirs::Eight),
            x => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid value {} for dirs", x),
            )),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum KeyValue {
    Version(f32),
    State(String),
    Dirs(Dirs),
    Frames(u32),
    Delay(Vec<f32>),
    Loop(bool),
    Rewind(bool),
    Movement(bool),
    Hotspot(Vec<f32>),
    Unk(String, Value),
}

pub fn key_value(input: &str) -> IResult<&str, KeyValue> {
    map_opt(
        separated_pair(key, tag(" = "), atom),
        |(key, value)| match (key, value) {
            (Key::Version, Value::Float(x)) => Some(KeyValue::Version(x)),
            (Key::State, Value::String(x)) => Some(KeyValue::State(x)),
            (Key::Dirs, Value::Int(x)) => Some(KeyValue::Dirs(x.try_into().ok()?)),
            (Key::Frames, Value::Int(x)) => Some(KeyValue::Frames(x)),
            (Key::Delay, Value::List(x)) => Some(KeyValue::Delay(x)),
            (Key::Loop, Value::Int(x)) => Some(KeyValue::Loop(x > 0)),
            (Key::Rewind, Value::Int(x)) => Some(KeyValue::Rewind(x > 0)),
            (Key::Movement, Value::Int(x)) => Some(KeyValue::Movement(x > 0)),
            (Key::Hotspot, Value::List(x)) => Some(KeyValue::Hotspot(x)),
            (Key::Unk(key), atom) => Some(KeyValue::Unk(key, atom)),
            _ => None,
        },
    )(input)
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
        assert_eq!(
            key_value(r#"dirs = 4"#),
            Ok(("", (KeyValue::Dirs(Dirs::Four))))
        );
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
