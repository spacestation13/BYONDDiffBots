use std::collections::HashMap;

use nom::{
    bytes::complete::tag,
    character::complete::{multispace0, newline, space1},
    combinator::{all_consuming, map_res, verify},
    multi::{many0, many1},
    sequence::{delimited, pair, terminated},
    IResult,
};

use super::{
    key_value::{key_value, KeyValue},
    state::{state, State},
    values::Value,
};

pub fn begin_dmi(input: &str) -> IResult<&str, &str> {
    terminated(tag("# BEGIN DMI"), newline)(input)
}

pub fn end_dmi(input: &str) -> IResult<&str, &str> {
    terminated(tag("# END DMI"), multispace0)(input)
}

#[derive(Debug)]
pub struct Header {
    pub version: f32,
    pub width: u32,
    pub height: u32,
    pub unk: Option<HashMap<String, Value>>,
}

impl TryFrom<(KeyValue, Vec<KeyValue>)> for Header {
    // TODO: anyhow
    type Error = std::io::Error;

    fn try_from((state, kvs): (KeyValue, Vec<KeyValue>)) -> Result<Self, Self::Error> {
        use std::io::{Error, ErrorKind};
        let version = match state {
            KeyValue::Version(version) => version,
            _ => unreachable!(),
        };

        if version != 4.0 {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("Version {} not supported, only 4.0", version),
            ));
        }

        let mut width = None;
        let mut height = None;
        let mut unk: Option<HashMap<String, Value>> = None;

        for value in kvs {
            match value {
                KeyValue::Width(w) => {
                    width = Some(w);
                }
                KeyValue::Height(h) => {
                    height = Some(h);
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
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        format!("{:?} not allowed here", x),
                    ));
                }
            }
        }

        Ok(Header {
            version,
            width: width.ok_or_else(|| Error::new(ErrorKind::InvalidData, "Never found width"))?,
            height: height
                .ok_or_else(|| Error::new(ErrorKind::InvalidData, "Never found height"))?,
            unk,
        })
    }
}

pub fn header(input: &str) -> IResult<&str, Header> {
    map_res(
        pair(
            verify(terminated(key_value, newline), |v| {
                matches!(v, KeyValue::Version(_))
            }),
            many1(delimited(space1, key_value, newline)),
        ),
        |(version, properties)| Header::try_from((version, properties)),
    )(input)
}

#[derive(Debug)]
pub struct Metadata {
    pub header: Header,
    pub states: Vec<State>,
}

impl Metadata {
    pub fn load<S: AsRef<str>>(input: S) -> Result<Metadata, std::io::Error> {
        Ok(metadata(input.as_ref())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?
            .1)
    }
}

pub fn metadata(input: &str) -> IResult<&str, Metadata> {
    let (tail, (header, states)) =
        all_consuming(delimited(begin_dmi, pair(header, many0(state)), end_dmi))(input)?;
    Ok((tail, Metadata { header, states }))
}

#[cfg(test)]
mod tests {
    use crate::parser::{key_value::Dirs, state::Frames};

    use super::*;
    #[test]
    fn test_metadata() {
        let description = r#"
# BEGIN DMI
version = 4.0
    width = 32
    height = 32
state = "state1"
    dirs = 4
    frames = 2
    delay = 1.2,1
    movement = 1
    loop = 1
    rewind = 0
    hotspot = 12,13,0
    future = "lmao"
state = "state2"
    dirs = 1
    frames = 1
# END DMI
"#
        .trim();

        let (tail, metadata) = metadata(description).unwrap();
        assert_eq!(tail, "");

        assert_eq!(metadata.header.version, 4.0);
        assert_eq!(metadata.header.width, 32);
        assert_eq!(metadata.header.height, 32);

        assert_eq!(metadata.states[0].name, "state1".to_string());
        assert_eq!(metadata.states[0].dirs, Dirs::Four);
        assert_eq!(
            metadata.states[0].frames,
            Frames::Delays(Vec::from([1.2, 1.0]))
        );
        assert!(metadata.states[0].movement);
        assert!(metadata.states[0].r#loop);
        assert!(!metadata.states[0].rewind);
        assert_eq!(metadata.states[0].hotspot, Some([12.0, 13.0, 0.0]));

        assert_eq!(metadata.states[1].name, "state2".to_string());
        assert_eq!(metadata.states[1].dirs, Dirs::One);
        assert_eq!(metadata.states[1].frames, Frames::One);

        dbg!(metadata);
    }
}
