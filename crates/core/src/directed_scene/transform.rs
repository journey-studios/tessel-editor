//! 3D transforms of the scene graph: TRS components and column-major 4x4 matrices.
//!
//! The layout and the `compose` formula are those of Three.js `Matrix4` (`elements`,
//! column-major), because the origin computes its effective matrices with it
//! (`scene-graph.ts: computeWorldMatrices`) and the equivalence oracle compares them.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    pub const ONE: Self = Self {
        x: 1.0,
        y: 1.0,
        z: 1.0,
    };

    pub fn distance(self, other: Self) -> f64 {
        (self.x - other.x)
            .hypot(self.y - other.y)
            .hypot(self.z - other.z)
    }
}

/// A rotation as stored in the bundle. Not normalized by the importer: the origin only checks
/// that its length is near 1 and composes it as given.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub w: f64,
}

impl Quat {
    pub const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    pub fn length(self) -> f64 {
        self.x.hypot(self.y).hypot(self.z).hypot(self.w)
    }
}

/// Position, rotation and scale relative to the parent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Transform {
    pub const IDENTITY: Self = Self {
        position: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn matrix(&self) -> Mat4 {
        Mat4::compose(self.position, self.rotation, self.scale)
    }
}

/// Column-major 4x4 matrix: `elements[12..15]` is the translation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat4(pub [f64; 16]);

impl Mat4 {
    pub const IDENTITY: Self = Self([
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]);

    /// Three.js `Matrix4.compose(position, quaternion, scale)`.
    pub fn compose(position: Vec3, rotation: Quat, scale: Vec3) -> Self {
        let Quat { x, y, z, w } = rotation;
        let (x2, y2, z2) = (x + x, y + y, z + z);
        let (xx, xy, xz) = (x * x2, x * y2, x * z2);
        let (yy, yz, zz) = (y * y2, y * z2, z * z2);
        let (wx, wy, wz) = (w * x2, w * y2, w * z2);
        Self([
            (1.0 - (yy + zz)) * scale.x,
            (xy + wz) * scale.x,
            (xz - wy) * scale.x,
            0.0,
            (xy - wz) * scale.y,
            (1.0 - (xx + zz)) * scale.y,
            (yz + wx) * scale.y,
            0.0,
            (xz + wy) * scale.z,
            (yz - wx) * scale.z,
            (1.0 - (xx + yy)) * scale.z,
            0.0,
            position.x,
            position.y,
            position.z,
            1.0,
        ])
    }

    /// `self * other` (apply `other` first, then `self`), as Three.js `Matrix4.multiply`.
    // WHY not `std::ops::Mul`: a product of finite matrices can overflow to infinity; callers
    // check `is_finite` on the result explicitly, which an operator would hide.
    #[allow(clippy::should_implement_trait)]
    pub fn mul(&self, other: &Self) -> Self {
        let (a, b) = (&self.0, &other.0);
        let mut out = [0.0; 16];
        for column in 0..4 {
            for row in 0..4 {
                out[column * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[column * 4 + k]).sum();
            }
        }
        Self(out)
    }

    pub fn transform_point(&self, point: Vec3) -> Vec3 {
        let m = &self.0;
        Vec3 {
            x: m[0] * point.x + m[4] * point.y + m[8] * point.z + m[12],
            y: m[1] * point.x + m[5] * point.y + m[9] * point.z + m[13],
            z: m[2] * point.x + m[6] * point.y + m[10] * point.z + m[14],
        }
    }

    pub fn translation(&self) -> Vec3 {
        Vec3 {
            x: self.0[12],
            y: self.0[13],
            z: self.0[14],
        }
    }

    pub fn is_finite(&self) -> bool {
        self.0.iter().all(|value| value.is_finite())
    }

    /// Largest absolute difference between two matrices' elements.
    pub fn max_abs_diff(&self, other: &Self) -> f64 {
        self.0
            .iter()
            .zip(other.0.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max)
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;

    #[test]
    fn compose_places_translation_and_scale() {
        let m = Mat4::compose(
            Vec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            Quat::IDENTITY,
            Vec3 {
                x: 2.0,
                y: 3.0,
                z: 4.0,
            },
        );
        let p = m.transform_point(Vec3::ONE);
        assert_eq!((p.x, p.y, p.z), (3.0, 5.0, 7.0));
    }

    #[test]
    fn rotation_about_z_quarter_turn_maps_x_to_y() {
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let m = Mat4::compose(
            Vec3::ZERO,
            Quat {
                x: 0.0,
                y: 0.0,
                z: half,
                w: half,
            },
            Vec3::ONE,
        );
        let p = m.transform_point(Vec3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        });
        assert!((p.x).abs() < 1e-12 && (p.y - 1.0).abs() < 1e-12 && p.z.abs() < 1e-12);
    }

    #[test]
    fn product_applies_the_right_operand_first() {
        let move_x = Mat4::compose(
            Vec3 {
                x: 5.0,
                y: 0.0,
                z: 0.0,
            },
            Quat::IDENTITY,
            Vec3::ONE,
        );
        let scale = Mat4::compose(
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3 {
                x: 2.0,
                y: 2.0,
                z: 2.0,
            },
        );
        // parent scales, child moves: the child's 5 becomes 10.
        assert_eq!(scale.mul(&move_x).translation().x, 10.0);
        assert_eq!(move_x.mul(&scale).translation().x, 5.0);
        assert_eq!(Mat4::IDENTITY.mul(&move_x), move_x);
    }
}
