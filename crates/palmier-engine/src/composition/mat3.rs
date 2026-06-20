//! `Mat3` — a 2-D affine transform as a 3×3 column-major matrix.
//!
//! The composition graph (E5-S3/S4) is **presentation-agnostic**: it must carry a
//! per-layer affine without depending on wgpu (the GPU device/textures land with
//! E5-S8, reconciliation #23). So we define our own minimal affine type here rather
//! than pulling `wgpu`/`glam` into a pure-assembly story. E5-S8 converts this to
//! whatever the GPU pass wants (e.g. a `mat3x3<f32>`/`mat4` uniform).
//!
//! ## Parity with the reference `CGAffineTransform`
//!
//! The macOS reference composes layer transforms with Core Graphics'
//! `CGAffineTransform` (`Sources/PalmierPro/Preview/CompositionBuilder.swift`
//! `affineTransform(for:natSize:renderSize:)`). A `CGAffineTransform` is the 2×3
//! matrix `[a b c d tx ty]` acting on a **row** vector:
//! `(x', y') = (a·x + c·y + tx, b·x + d·y + ty)`.
//!
//! We store the same six coefficients (`a, b, c, d, tx, ty`) and define
//! [`Mat3::concatenating`] with the **same operand order** as CG's
//! `t1.concatenating(t2)` (apply `t1` first, then `t2`), so the port is verbatim:
//! every `A.concatenating(B)` in the reference maps 1:1 to `a.concatenating(b)`
//! here. This is the load-bearing detail — CG concatenation is `t1 * t2` in
//! row-vector convention, which is the *reverse* of the usual column-vector
//! `B * A`. Keeping CG's order means the ported math needs no rewriting.

/// A 2-D affine transform, stored as the six `CGAffineTransform` coefficients
/// `[a b c d tx ty]` operating on a row vector `(x, y, 1)`:
///
/// ```text
/// x' = a·x + c·y + tx
/// y' = b·x + d·y + ty
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat3 {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub tx: f64,
    pub ty: f64,
}

impl Default for Mat3 {
    fn default() -> Self {
        Mat3::IDENTITY
    }
}

impl Mat3 {
    /// The identity transform.
    pub const IDENTITY: Mat3 = Mat3 {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// A pure scale `(sx, sy)` (CG `CGAffineTransform(scaleX:y:)`).
    pub fn scale(sx: f64, sy: f64) -> Mat3 {
        Mat3 {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// A pure translation `(tx, ty)` (CG `CGAffineTransform(translationX:y:)`).
    pub fn translation(tx: f64, ty: f64) -> Mat3 {
        Mat3 {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx,
            ty,
        }
    }

    /// A pure rotation by `radians` (CG `CGAffineTransform(rotationAngle:)`):
    /// `[cos sin -sin cos 0 0]`.
    pub fn rotation(radians: f64) -> Mat3 {
        let (s, co) = radians.sin_cos();
        Mat3 {
            a: co,
            b: s,
            c: -s,
            d: co,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// `self.concatenating(other)` in **Core Graphics order**: apply `self`
    /// first, then `other`. Equivalent to the row-vector product `self * other`
    /// (CG `CGAffineTransform.concatenating(_:)`).
    ///
    /// Concretely, for a point `p`: `p · (self.concatenating(other))
    /// == (p · self) · other`.
    pub fn concatenating(self, other: Mat3) -> Mat3 {
        // Row-vector matrix product. With CG's layout
        //   M = | a  b  0 |
        //       | c  d  0 |
        //       | tx ty 1 |
        // and p·M, `self.concatenating(other)` is the matrix M_self * M_other.
        Mat3 {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            tx: self.tx * other.a + self.ty * other.c + other.tx,
            ty: self.tx * other.b + self.ty * other.d + other.ty,
        }
    }

    /// Apply this transform to a point, returning `(x', y')`.
    pub fn apply(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.tx,
            self.b * x + self.d * y + self.ty,
        )
    }

    /// The determinant `a·d − b·c`.
    pub fn determinant(self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    /// The inverse transform, or `None` when singular (determinant ≈ 0).
    /// Matches CG `CGAffineTransform.inverted()` (which returns the original when
    /// non-invertible — here we surface that as `None` so callers decide).
    pub fn inverted(self) -> Option<Mat3> {
        let det = self.determinant();
        if det.abs() < 1e-12 {
            return None;
        }
        let inv_det = 1.0 / det;
        let a = self.d * inv_det;
        let b = -self.b * inv_det;
        let c = -self.c * inv_det;
        let d = self.a * inv_det;
        Some(Mat3 {
            a,
            b,
            c,
            d,
            tx: -(self.tx * a + self.ty * c),
            ty: -(self.tx * b + self.ty * d),
        })
    }

    /// Column-major `[f32; 9]` for a GPU `mat3x3<f32>` upload (E5-S8). Columns are
    /// `(a, b, 0)`, `(c, d, 0)`, `(tx, ty, 1)` — the standard expansion of the
    /// 2×3 affine to a 3×3 matrix.
    pub fn to_cols_array_f32(self) -> [f32; 9] {
        [
            self.a as f32,
            self.b as f32,
            0.0,
            self.c as f32,
            self.d as f32,
            0.0,
            self.tx as f32,
            self.ty as f32,
            1.0,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: Mat3, b: Mat3) -> bool {
        let e = 1e-9;
        (a.a - b.a).abs() < e
            && (a.b - b.b).abs() < e
            && (a.c - b.c).abs() < e
            && (a.d - b.d).abs() < e
            && (a.tx - b.tx).abs() < e
            && (a.ty - b.ty).abs() < e
    }

    #[test]
    fn identity_is_neutral() {
        let m = Mat3::scale(2.0, 3.0);
        assert!(approx(m.concatenating(Mat3::IDENTITY), m));
        assert!(approx(Mat3::IDENTITY.concatenating(m), m));
        assert_eq!(Mat3::IDENTITY.apply(7.0, 9.0), (7.0, 9.0));
    }

    #[test]
    fn scale_then_translate_matches_cg_order() {
        // CG: scale(2,2).concatenating(translation(10,20)) applied to (1,1):
        // scale first → (2,2), then translate → (12,22).
        let m = Mat3::scale(2.0, 2.0).concatenating(Mat3::translation(10.0, 20.0));
        assert_eq!(m.apply(1.0, 1.0), (12.0, 22.0));
    }

    #[test]
    fn translate_then_scale_differs_from_reverse() {
        // translation(10,0).concatenating(scale(2,2)) applied to (0,0):
        // translate → (10,0), then scale → (20,0).
        let m = Mat3::translation(10.0, 0.0).concatenating(Mat3::scale(2.0, 2.0));
        assert_eq!(m.apply(0.0, 0.0), (20.0, 0.0));
    }

    #[test]
    fn rotation_90_degrees() {
        // Rotate (1,0) by 90° → (0,1) in CG's convention (b = sin).
        let m = Mat3::rotation(std::f64::consts::FRAC_PI_2);
        let (x, y) = m.apply(1.0, 0.0);
        assert!(x.abs() < 1e-9);
        assert!((y - 1.0).abs() < 1e-9);
    }

    #[test]
    fn inverse_round_trips() {
        let m = Mat3 {
            a: 2.0,
            b: 0.5,
            c: -0.3,
            d: 1.5,
            tx: 4.0,
            ty: -2.0,
        };
        let inv = m.inverted().unwrap();
        let back = m.concatenating(inv);
        assert!(approx(back, Mat3::IDENTITY), "M·M⁻¹ should be identity: {back:?}");
        // Singular matrix → None.
        assert!(Mat3::scale(0.0, 0.0).inverted().is_none());
    }

    #[test]
    fn cols_array_expands_affine() {
        let m = Mat3 {
            a: 1.0,
            b: 2.0,
            c: 3.0,
            d: 4.0,
            tx: 5.0,
            ty: 6.0,
        };
        assert_eq!(
            m.to_cols_array_f32(),
            [1.0, 2.0, 0.0, 3.0, 4.0, 0.0, 5.0, 6.0, 1.0]
        );
    }
}
