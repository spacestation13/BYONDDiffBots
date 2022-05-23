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
    pub other: Option<HashMap<String, Value>>,
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
        let mut other: Option<HashMap<String, Value>> = None;

        for value in kvs {
            match value {
                KeyValue::Width(w) => {
                    width = Some(w);
                }
                KeyValue::Height(h) => {
                    height = Some(h);
                }
                KeyValue::Unk(key, value) => {
                    if let Some(map) = &mut other {
                        map.insert(key, value);
                    } else {
                        let mut new_map = HashMap::new();
                        new_map.insert(key, value);
                        other = Some(new_map);
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
            other,
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

pub fn metadata(input: &str) -> IResult<&str, Metadata> {
    let (tail, (header, states)) =
        all_consuming(delimited(begin_dmi, pair(header, many0(state)), end_dmi))(input)?;
    Ok((tail, Metadata { header, states }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_metadata() {
        let description = r#"
# BEGIN DMI
version = 4.0
    width = 32
    height = 32
state = "duplicate"
    dirs = 1
    frames = 1
state = "duplicate"
    dirs = 1
    frames = 1
state = "duplicate"
    dirs = 1
    frames = 1
# END DMI
"#
        .trim();

        let (tail, metadata) = metadata(description).unwrap();
        assert_eq!(tail, "");

        dbg!(metadata);
    }
}
