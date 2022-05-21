use nom::{
    character::complete::tab,
    combinator::verify,
    multi::many1,
    sequence::{pair, preceded},
    IResult,
};

use crate::parser::atoms::Atom;

use super::key_value::{self, key_value};

#[derive(Debug, Default)]
pub struct State {
    pub name: String,
    pub width: u32,
    pub height: u32,
    // dirs: Dirs,
    // frames: Frames,
}

pub fn state(input: &str) -> IResult<&str, State> {
    let (rest, (state_kv, kvs)) = pair(
        verify(key_value, |v| v.0 == "state"),
        many1(preceded(tab, key_value)),
    )(input)?;

    let mut new_state = State::default();
    if let Atom::String(name) = state_kv.1 {
        new_state.name = name;
    }

    for (key, value) in kvs {
        match key {
            _ => unimplemented!(),
        }
    }

    todo!()
}

#[cfg(test)]
mod tests {
    #[test]
    fn metadata() {
        let description = r#"
state = "duplicate"
    dirs = 1
    frames = 1
"#
        .trim();
    }

    #[test]
    fn true_bullshit() {
        let description = r#"
state = "bluespace_coffee"
    dirs = 1
    frames = 4
    delay = 1,2,5.4,3
"#
        .trim();
    }
}
