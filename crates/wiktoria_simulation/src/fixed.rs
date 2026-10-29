//! The game's fixed-point numbers: signed 64-bit with 15 fractional bits (1.0 = 32768),
//! FLD_FIXED_POINT_FRACTION_BITS. Arithmetic wraps like the original 64-bit integer code.

use std::fmt;

/// Number of fractional bits. `Fixed::check_matrix` verifies the rules file agrees.
pub const FRAC_BITS: u32 = 15;
const SCALE: f64 = (1u64 << FRAC_BITS) as f64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Fixed(pub i64);

impl Fixed {
    pub const ZERO: Fixed = Fixed(0);
    pub const ONE: Fixed = Fixed(1 << FRAC_BITS);

    pub fn check_matrix(m: &wiktoria_data::engine_rules::EngineRules) -> Result<(), String> {
        let bits = crate::int_field(m, "FLD_FIXED_POINT_FRACTION_BITS")?;
        if bits != i64::from(FRAC_BITS) {
            return Err(format!("rules file says {bits} fraction bits, the simulation is built for {FRAC_BITS}"));
        }
        Ok(())
    }

    /// Conversion from a decimal the way the game converts its constants and script numbers:
    /// scale, add 0.5 away from zero, truncate.
    pub fn from_f64(x: f64) -> Fixed {
        let half = if x < 0.0 { -0.5 } else { 0.5 };
        Fixed((x * SCALE + half) as i64)
    }

    /// Same, for a value the game holds as a single-precision float.
    pub fn from_f32(x: f32) -> Fixed {
        Fixed::from_f64(f64::from(x))
    }

    /// A number as printed in a save (5 decimals of a fixed-point value): the nearest raw value.
    pub fn from_save_text(s: &str) -> Option<Fixed> {
        let x: f64 = s.trim().parse().ok()?;
        Some(Fixed((x * SCALE).round() as i64))
    }

    pub fn to_f64(self) -> f64 {
        self.0 as f64 / SCALE
    }

    /// Quotient: shift the dividend left by the fraction bits, then divide, truncating.
    /// `None` when dividing by zero.
    pub fn checked_div(self, o: Fixed) -> Option<Fixed> {
        (o.0 != 0).then(|| Fixed(self.0.wrapping_shl(FRAC_BITS).wrapping_div(o.0)))
    }

    /// Square root through floating point, rounded back half away from zero.
    pub fn sqrt(self) -> Fixed {
        Fixed::from_f64((self.0 as f64 / SCALE).sqrt())
    }
}

impl std::ops::Add for Fixed {
    type Output = Fixed;
    fn add(self, o: Fixed) -> Fixed {
        Fixed(self.0.wrapping_add(o.0))
    }
}

impl std::ops::Sub for Fixed {
    type Output = Fixed;
    fn sub(self, o: Fixed) -> Fixed {
        Fixed(self.0.wrapping_sub(o.0))
    }
}

/// Product: 64-bit multiply, then shift right by the fraction bits.
#[allow(clippy::suspicious_arithmetic_impl)] // fixed-point multiply is a shifted integer multiply
impl std::ops::Mul for Fixed {
    type Output = Fixed;
    fn mul(self, o: Fixed) -> Fixed {
        Fixed(self.0.wrapping_mul(o.0) >> FRAC_BITS)
    }
}

impl std::ops::Neg for Fixed {
    type Output = Fixed;
    fn neg(self) -> Fixed {
        Fixed(self.0.wrapping_neg())
    }
}

impl fmt::Display for Fixed {
    /// Five decimals, like the game's saves.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.5}", self.to_f64())
    }
}
