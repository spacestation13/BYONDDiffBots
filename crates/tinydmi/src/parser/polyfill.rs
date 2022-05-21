use nom::{
    error::{ErrorKind, ParseError},
    Err, IResult, InputLength, Parser,
};

/// it's [`nom::separated_list1`] except it fails if the first separator isn't found
pub fn separated_list1_nonoptional<I, O, O2, E, F, G>(
    mut sep: G,
    mut f: F,
) -> impl FnMut(I) -> IResult<I, Vec<O>, E>
where
    I: Clone + InputLength,
    F: Parser<I, O, E>,
    G: Parser<I, O2, E>,
    E: ParseError<I>,
{
    move |mut i: I| {
        let mut res = Vec::new();

        // Parse the first element
        match f.parse(i.clone()) {
            Err(e) => return Err(e),
            Ok((i1, o)) => {
                res.push(o);
                i = i1;
            }
        }

        // MUST have the first separator
        if let Err(e) = sep.parse(i.clone()) {
            return Err(e);
        };

        loop {
            let len = i.input_len();
            match sep.parse(i.clone()) {
                Err(Err::Error(_)) => return Ok((i, res)),
                Err(e) => return Err(e),
                Ok((i1, _)) => {
                    // infinite loop check: the parser must always consume
                    if i1.input_len() == len {
                        return Err(Err::Error(E::from_error_kind(i1, ErrorKind::SeparatedList)));
                    }

                    match f.parse(i1.clone()) {
                        Err(Err::Error(_)) => return Ok((i, res)),
                        Err(e) => return Err(e),
                        Ok((i2, o)) => {
                            res.push(o);
                            i = i2;
                        }
                    }
                }
            }
        }
    }
}
