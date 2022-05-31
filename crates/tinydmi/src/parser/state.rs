use std::collections::HashMap;

use anyhow::{format_err, Context};
use nom::{
    character::complete::{newline, space1},
    combinator::{map_res, verify},
    multi::many1,
    sequence::{delimited, pair, terminated},
    IResult,
};

use super::{
    key_value::{key_value, Dirs, KeyValue},
    values::Value,
};

#[derive(Clone, Debug, PartialEq)]
pub enum Frames {
    One,
    Count(u32),
    Delays(Vec<f32>),
}

impl Into<u32> for Frames {
    fn into(self) -> u32 {
        match self {
            Frames::One => 1,
            Frames::Count(u) => u,
            Frames::Delays(v) => v.len() as u32,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub name: String,
    pub dirs: Dirs,
    pub frames: Frames,
    pub r#loop: bool,
    pub rewind: bool,
    pub movement: bool,
    pub hotspot: Option<[f32; 3]>,
    pub unk: Option<HashMap<String, Value>>,
}

impl TryFrom<(KeyValue, Vec<KeyValue>)> for State {
    type Error = anyhow::Error;

    fn try_from((state, kvs): (KeyValue, Vec<KeyValue>)) -> Result<Self, Self::Error> {
        let name = match state {
            KeyValue::State(name) => name,
            _ => unreachable!(),
        };

        let mut dirs = None;
        let mut frames = Frames::One;
        let mut r#loop = false;
        let mut rewind = false;
        let mut movement = false;
        let mut hotspot = None;
        let mut unk: Option<HashMap<String, Value>> = None;

        for kv in kvs {
            match kv {
                KeyValue::Dirs(d) => dirs = Some(d),
                KeyValue::Frames(f) => {
                    if matches!(frames, Frames::One) {
                        if f == 1 {
                            frames = Frames::One
                        } else {
                            frames = Frames::Count(f);
                        }
                    } else {
                        return Err(format_err!("Found `frames` in illegal position"));
                    }
                }
                KeyValue::Delay(f) => {
                    if matches!(frames, Frames::Count(_)) {
                        frames = Frames::Delays(f)
                    } else {
                        return Err(format_err!("Found `delay` key without `frames` key"));
                    }
                }
                KeyValue::Loop(do_loop) => r#loop = do_loop,
                KeyValue::Rewind(do_rewind) => rewind = do_rewind,
                KeyValue::Movement(do_movement) => movement = do_movement,
                KeyValue::Hotspot(h) => {
                    if h.len() == 3 {
                        let mut buf = [0.0; 3];
                        buf.copy_from_slice(&h[0..3]);
                        hotspot = Some(buf);
                    } else {
                        return Err(format_err!("Hotspot information was not length 3"));
                    }
                }
                KeyValue::Unk(key, value) => {
                    if let Some(map) = &mut unk {
                        map.insert(key, value);
                    } else {
                        let mut new_map = HashMap::new();
                        new_map.insert(key, value);
                        unk = Some(new_map);
                    }
                }
                x => {
                    return Err(format_err!("{:?} not allowed here", x));
                }
            }
        }

        Ok(State {
            name,
            dirs: dirs.context("Required field `dirs` was not found")?,
            frames,
            r#loop,
            rewind,
            movement,
            hotspot,
            unk,
        })
    }
}

pub fn state(input: &str) -> IResult<&str, State> {
    map_res(
        pair(
            verify(terminated(key_value, newline), |v| {
                matches!(v, super::key_value::KeyValue::State(_))
            }),
            many1(delimited(space1, key_value, newline)),
        ),
        |(state_name, properties)| State::try_from((state_name, properties)),
    )(input)
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn metadata() {
        let description = r#"
state = "duplicate"
    dirs = 1
    frames = 1
"#
        .trim();

        let (_, state) = state(description).unwrap();
        assert_eq!(state.dirs, Dirs::One);
        assert_eq!(state.frames, Frames::One);
        assert_eq!(state.name, "duplicate");
    }

    #[test]
    fn delay() {
        let description = r#"
state = "bluespace_coffee"
    dirs = 1
    frames = 4
    delay = 1,2,5.4,3
state = "..."
"#
        .trim();

        let (tail, state) = state(description).unwrap();
        assert_eq!(tail, r#"state = "...""#);
        assert_eq!(state.dirs, Dirs::One);
        assert_eq!(
            state.frames,
            Frames::Delays(Vec::from([1.0, 2.0, 5.4, 3.0]))
        );
        assert_eq!(state.name, "bluespace_coffee");
    }

    #[test]
    fn fail_delay_without_frames() {
        let description = r#"
state = "bluespace_coffee"
    dirs = 1
    delay = 1,2,5.4,3
state = "..."
        "#
        .trim();

        let x = state(description);
        assert!(matches!(x, Err(_)));
    }
}
