// use nom::{
//     character::complete::{newline, space1},
//     combinator::verify,
//     multi::many1,
//     sequence::{delimited, pair, terminated},
//     IResult,
// };

// use crate::parser::atoms::Atom;

// use super::key_value::key_value;

// #[derive(Debug, Default, PartialEq, Eq)]
// pub enum Dirs {
//     #[default]
//     One,
//     Four,
//     Eight,
// }

// fn parse_dirs(input: &str, dirs: u32) -> IResult<&str, Dirs> {
//     match dirs {
//         1 => Ok(("", Dirs::One)),
//         4 => Ok(("", Dirs::Four)),
//         8 => Ok(("", Dirs::Eight)),
//         _ => Err(nom::Err::Failure(nom::error::Error {
//             input,
//             code: nom::error::ErrorKind::Alt,
//         })),
//     }
// }

// #[derive(Debug, Default, PartialEq)]
// pub enum Frames {
//     #[default]
//     One,
//     Count(u32),
//     Delays(Vec<f32>),
// }

// #[derive(Debug, Default)]
// pub struct State {
//     pub name: String,
//     pub width: u32,
//     pub height: u32,
//     pub dirs: Dirs,
//     pub frames: Frames,
//     pub r#loop: bool,
//     pub rewind: bool,
//     pub movement: bool,
// }

// pub fn state(input: &str) -> IResult<&str, State> {
//     let (rest, (state_kv, kvs)) = pair(
//         verify(terminated(key_value, newline), |v| v.0 == "state"),
//         many1(delimited(space1, key_value, newline)),
//     )(input)?;

//     let mut new_state = State::default();
//     if let Atom::String(name) = state_kv.1 {
//         new_state.name = name;
//     }

//     for (key, value) in kvs {
//         match (key, value) {
//             ("width", Atom::Int(width)) => new_state.width = width,
//             ("height", Atom::Int(height)) => new_state.height = height,
//             ("dirs", Atom::Int(dirs)) => new_state.dirs = parse_dirs(input, dirs)?.1,
//             ("frames", Atom::Int(frames)) => {
//                 match new_state.frames {
//                     Frames::One => {}
//                     _ => continue,
//                 }
//                 new_state.frames = Frames::Count(frames);
//             }
//             ("delay", Atom::List(mut vec)) => match new_state.frames {
//                 Frames::One => {
//                     if vec.iter().all(|&n| n == 1.) {
//                         new_state.frames = Frames::Count(vec.len() as u32)
//                     } else {
//                         new_state.frames = Frames::Delays(vec);
//                     }
//                 }
//                 Frames::Count(n) => {
//                     if !vec.iter().all(|&n| n == 1.) {
//                         vec.truncate(n as usize);
//                         new_state.frames = Frames::Delays(vec);
//                     }
//                 }
//                 Frames::Delays(_) => continue,
//             },
//             ("loop", Atom::Int(should_loop)) => new_state.r#loop = should_loop > 0,
//             ("rewind", Atom::Int(should_rewind)) => new_state.rewind = should_rewind > 0,
//             ("hotspot", _) => {}
//             ("movement", Atom::Int(should_movement)) => new_state.movement = should_movement > 0,
//             _ => unimplemented!(),
//         }
//     }

//     Ok((rest, new_state))
// }

// #[cfg(test)]
// mod tests {

//     use super::*;

//     #[test]
//     fn metadata() {
//         let description = r#"
// state = "duplicate"
//     dirs = 1
//     frames = 1
// "#
//         .trim();

//         let (_, state) = state(description).unwrap();
//         assert_eq!(state.dirs, Dirs::One);
//         assert_eq!(state.frames, Frames::One);
//         assert_eq!(state.name, "duplicate");
//     }

//     #[test]
//     fn true_bullshit() {
//         let description = r#"
// state = "bluespace_coffee"
//     dirs = 1
//     frames = 4
//     delay = 1,2,5.4,3
// "#
//         .trim();

//         let (_, state) = state(description).unwrap();
//         assert_eq!(state.dirs, Dirs::One);
//         assert_eq!(
//             state.frames,
//             Frames::Delays(Vec::from([1.0, 2.0, 5.4, 3.0]))
//         );
//         assert_eq!(state.name, "bluespace_coffee");
//     }
// }
