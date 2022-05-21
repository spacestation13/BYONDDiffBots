use nom::{bytes::complete::tag, combinator::recognize, IResult};

pub fn begin_dmi(input: &str) -> IResult<&str, &str> {
    tag("# BEGIN DMI")(input)
}

pub fn end_dmi(input: &str) -> IResult<&str, &str> {
    tag("# END DMI")(input)
}

#[cfg(test)]
mod tests {
    #[test]
    fn metadata() {
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
    }
}
