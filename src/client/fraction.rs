//! Bounded constructor arithmetic used by native bundle weights. Keep the
//! returned representation: original Fraction.equals is not rational equality.
use anyhow::{Context, Result, bail};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Fraction {
    pub numerator: i32,
    pub denominator: i32,
}
impl Fraction {
    pub(crate) const ZERO: Self = Self {
        numerator: 0,
        denominator: 1,
    };
    pub(crate) fn new(mut numerator: i32, mut denominator: i32) -> Result<Self> {
        if denominator == 0 {
            bail!("native Fraction denominator must not be zero");
        }
        if denominator < 0 {
            numerator = numerator
                .checked_neg()
                .context("native Fraction negate overflow")?;
            denominator = denominator
                .checked_neg()
                .context("native Fraction negate overflow")?;
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }
    pub(crate) fn multiply(self, other: Self) -> Result<Self> {
        if self.numerator == 0 || other.numerator == 0 {
            return Ok(Self::ZERO);
        }
        let first = gcd(i64::from(self.numerator), i64::from(other.denominator)) as i32;
        let second = gcd(i64::from(other.numerator), i64::from(self.denominator)) as i32;
        let numerator = (self.numerator / first)
            .checked_mul(other.numerator / second)
            .context("native Fraction numerator multiply overflow")?;
        let denominator = (self.denominator / second)
            .checked_mul(other.denominator / first)
            .context("native Fraction denominator multiply overflow")?;
        let divisor = gcd(i64::from(numerator), i64::from(denominator)) as i32;
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }
    pub(crate) fn add(self, other: Self) -> Result<Self> {
        if self.numerator == 0 {
            return Ok(other);
        }
        if other.numerator == 0 {
            return Ok(self);
        }
        let first = gcd(i64::from(self.denominator), i64::from(other.denominator)) as i32;
        if first == 1 {
            let left = self
                .numerator
                .checked_mul(other.denominator)
                .context("native Fraction add multiply overflow")?;
            let right = other
                .numerator
                .checked_mul(self.denominator)
                .context("native Fraction add multiply overflow")?;
            let numerator = left
                .checked_add(right)
                .context("native Fraction add overflow")?;
            let denominator = self
                .denominator
                .checked_mul(other.denominator)
                .context("native Fraction denominator add overflow")?;
            return Ok(Self {
                numerator,
                denominator,
            });
        }
        // Original BigInteger intermediates fit i64 for two signed-i32 products
        // divided by positive denominators. The returned numerator remains i32.
        let total = i64::from(self.numerator) * i64::from(other.denominator / first)
            + i64::from(other.numerator) * i64::from(self.denominator / first);
        let second = gcd(total.rem_euclid(i64::from(first)), i64::from(first));
        let numerator =
            i32::try_from(total / second).context("native Fraction add numerator overflow")?;
        let denominator = (self.denominator / first)
            .checked_mul(other.denominator / second as i32)
            .context("native Fraction denominator add overflow")?;
        Ok(Self {
            numerator,
            denominator,
        })
    }
}
fn gcd(left: i64, right: i64) -> i64 {
    let (mut left, mut right) = (left.abs(), right.abs());
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value as Json;
    use std::io::Read;
    #[test]
    fn original_fraction_constructors_arithmetic_representations_and_all_pairs_match() {
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../data/client_api/fraction_constructor_cases-1.21.11.json.gz")[..],
        )
        .read_to_end(&mut bytes)
        .unwrap();
        let facts: Json = serde_json::from_slice(&bytes).unwrap();
        let mut values = Vec::new();
        for row in facts["cases"].as_array().unwrap() {
            let a = row["arguments"].as_array().unwrap();
            let number = |i: usize| a[i].as_i64().unwrap() as i32;
            let value = Fraction::new(number(0), number(1)).and_then(|left| match row["operation"]
                .as_str()
                .unwrap()
            {
                "from" => Ok(left),
                "add" => left.add(Fraction::new(number(2), number(3))?),
                "multiply" => left.multiply(Fraction::new(number(2), number(3))?),
                _ => unreachable!(),
            });
            assert_eq!(
                value.is_ok(),
                row["accepted"].as_bool().unwrap(),
                "{row}: {value:?}"
            );
            values.push(value.ok().inspect(|value| {
                assert_eq!(
                    value.numerator,
                    row["numerator"].as_i64().unwrap() as i32,
                    "{row}"
                );
                assert_eq!(
                    value.denominator,
                    row["denominator"].as_i64().unwrap() as i32,
                    "{row}"
                );
            }));
        }
        assert_eq!(
            (values.len(), values.iter().filter(|v| v.is_some()).count()),
            (680, 454)
        );
        for pair in facts["pairs"].as_array().unwrap() {
            assert_eq!(
                values[pair["a"].as_u64().unwrap() as usize]
                    == values[pair["b"].as_u64().unwrap() as usize],
                pair["equal"].as_bool().unwrap(),
                "{pair}"
            );
        }
        assert_eq!(facts["pairs"].as_array().unwrap().len(), 103285);
    }
}
