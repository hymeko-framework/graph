//! Thin binary-stdio client of `BoundsIndex`, for callers in other languages.
//!
//! stdout starts with `SBS1`. Each request is little-endian u32 count followed
//! by count records (lo_x,lo_y,lo_z,hi_x,hi_y,hi_z), all f64. Response: u32 pair
//! count followed by (u32,u32) slots. Boxes are rebuilt for every request. This
//! adapter owns no geometry, IR, spatial partition or collision algorithm.
use std::io::{self, Read, Write};

use hymeko_sbsh::BoundsIndex;

const MAX_BOXES: usize = 4096;

fn serve(mut input: impl Read, mut output: impl Write) -> io::Result<()> {
    output.write_all(b"SBS1")?;
    output.flush()?;
    loop {
        let mut count = [0; 4];
        if input.read(&mut count[..1])? == 0 {
            return Ok(());
        }
        input.read_exact(&mut count[1..])?;
        let count = u32::from_le_bytes(count) as usize;
        if count > MAX_BOXES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "at most 4096 boxes per request",
            ));
        }
        let mut boxes = Vec::with_capacity(count);
        for _ in 0..count {
            let mut values = [0.; 6];
            for value in &mut values {
                let mut bytes = [0; 8];
                input.read_exact(&mut bytes)?;
                *value = f64::from_le_bytes(bytes);
            }
            if values.iter().any(|v| !v.is_finite()) || (0..3).any(|k| values[k] > values[k + 3]) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "finite ordered bounds required",
                ));
            }
            boxes.push((
                [values[0], values[1], values[2]],
                [values[3], values[4], values[5]],
            ));
        }
        let mut pairs = Vec::new();
        BoundsIndex::new(&boxes).for_each_overlap(|i, j| pairs.push((i as u32, j as u32)));
        output.write_all(&(pairs.len() as u32).to_le_bytes())?;
        for (i, j) in pairs {
            output.write_all(&i.to_le_bytes())?;
            output.write_all(&j.to_le_bytes())?;
        }
        output.flush()?;
    }
}

fn main() -> io::Result<()> {
    serve(io::stdin().lock(), io::BufWriter::new(io::stdout().lock()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(boxes: &[[f64; 6]]) -> Vec<u8> {
        let mut bytes = (boxes.len() as u32).to_le_bytes().to_vec();
        for bounds in boxes {
            for x in bounds {
                bytes.extend(x.to_le_bytes());
            }
        }
        bytes
    }

    #[test]
    fn empty_contact_and_multiple_requests() {
        let mut input = request(&[]);
        input.extend(request(&[
            [0., 0., 0., 1., 1., 0.],
            [1., 0., 0., 2., 1., 0.],
        ]));
        input.extend(request(&[
            [0., 0., 0., 1., 1., 0.],
            [2., 0., 0., 3., 1., 0.],
        ]));
        let mut output = Vec::new();
        serve(input.as_slice(), &mut output).unwrap();
        let mut expected = b"SBS1".to_vec();
        for value in [0u32, 1, 0, 1, 0] {
            expected.extend(value.to_le_bytes());
        }
        assert_eq!(output, expected);
    }

    #[test]
    fn invalid_and_truncated_requests_fail() {
        for bytes in [
            4097u32.to_le_bytes().to_vec(),
            vec![1],
            request(&[[0.; 6]])[..20].to_vec(),
            request(&[[2., 0., 0., 1., 1., 0.]]),
            request(&[[f64::NAN, 0., 0., 1., 1., 0.]]),
        ] {
            assert!(serve(bytes.as_slice(), Vec::new()).is_err());
        }
    }
}
