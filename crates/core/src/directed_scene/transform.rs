//! 3D transforms of the scene graph: TRS components and column-major 4x4 matrices.
//!
//! The layout and the `compose` formula are those of Three.js `Matrix4` (`elements`,
//! column-major), because the origin computes its effective matrices with it
//! (`scene-graph.ts: computeWorldMatrices`) and the equivalence oracle compares them.

use serde_json::{Value, json};

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

    /// `{ "x": .., "y": .., "z": .. }`, the bundle's own shape for a vector.
    pub fn to_json(self) -> Value {
        json!({ "x": self.x, "y": self.y, "z": self.z })
    }

    /// Component-wise sum (Three.js `Vector3.add`; also the origin's `addVec3`).
    pub fn plus(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
            z: self.z + other.z,
        }
    }

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

    /// Three.js `Euler.setFromQuaternion(q, "XYZ")`, in degrees (`quatSchemaToEulerDegrees`).
    ///
    /// WHY this exists: the origin does not keep a node's quaternion while it animates. It turns it
    /// into XYZ Euler degrees, adds the performance offset there and rebuilds the rotation from the
    /// sum, so the same path is needed here (not the quaternion composed directly) for the state to
    /// be the origin's. Like Three.js it assumes a unit quaternion (the importer checks the length)
    /// and, at a gimbal lock (`|m13| >= 0.9999999`), puts the whole turn in `x` and sets `z` to 0.
    pub fn to_euler_xyz_degrees(self) -> Vec3 {
        // `makeRotationFromQuaternion` is `compose(0, q, 1)`; the elements read below are m13, m23,
        // m33, m12, m11, m32 and m22 of its column-major array.
        let m = Mat4::compose(Vec3::ZERO, self, Vec3::ONE).0;
        let (m11, m12, m13) = (m[0], m[4], m[8]);
        let (m22, m23) = (m[5], m[9]);
        let (m32, m33) = (m[6], m[10]);
        let y = m13.clamp(-1.0, 1.0).asin();
        let (x, z) = if m13.abs() < 0.999_999_9 {
            ((-m23).atan2(m33), (-m12).atan2(m11))
        } else {
            (m32.atan2(m22), 0.0)
        };
        // `radToDeg = 180 / Math.PI`, computed once and multiplied, as the origin does.
        let rad_to_deg = 180.0 / std::f64::consts::PI;
        Vec3 {
            x: x * rad_to_deg,
            y: y * rad_to_deg,
            z: z * rad_to_deg,
        }
    }

    /// Three.js `Quaternion.setFromEuler(Euler(x, y, z, "XYZ"))` from degrees (`eulerDegToQuat`).
    pub fn from_euler_xyz_degrees(degrees: Vec3) -> Self {
        // The origin writes `(deg * Math.PI) / 180`: same operations, same order.
        let radians = |deg: f64| deg * std::f64::consts::PI / 180.0;
        let (x, y, z) = (radians(degrees.x), radians(degrees.y), radians(degrees.z));
        let (c1, c2, c3) = ((x / 2.0).cos(), (y / 2.0).cos(), (z / 2.0).cos());
        let (s1, s2, s3) = ((x / 2.0).sin(), (y / 2.0).sin(), (z / 2.0).sin());
        Self {
            x: s1 * c2 * c3 + c1 * s2 * s3,
            y: c1 * s2 * c3 - s1 * c2 * s3,
            z: c1 * c2 * s3 + s1 * s2 * c3,
            w: c1 * c2 * c3 - s1 * s2 * s3,
        }
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

    fn euler(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 { x, y, z }
    }

    fn rotation_of(q: Quat) -> Mat4 {
        Mat4::compose(Vec3::ZERO, q, Vec3::ONE)
    }

    #[test]
    fn a_quarter_turn_about_z_is_ninety_degrees_in_euler() {
        let q = Quat::from_euler_xyz_degrees(euler(0.0, 0.0, 90.0));
        let half = std::f64::consts::FRAC_1_SQRT_2;
        assert!((q.z - half).abs() < 1e-15 && (q.w - half).abs() < 1e-15);
        let back = q.to_euler_xyz_degrees();
        assert!(back.x.abs() < 1e-12 && back.y.abs() < 1e-12 && (back.z - 90.0).abs() < 1e-12);
    }

    #[test]
    fn euler_angles_survive_a_round_trip_away_from_the_poles() {
        for (x, y, z) in [
            (10.0, 20.0, 30.0),
            (-170.0, 45.0, 95.0),
            (0.0, -89.0, 0.0),
            (179.0, 0.5, -179.0),
        ] {
            let back = Quat::from_euler_xyz_degrees(euler(x, y, z)).to_euler_xyz_degrees();
            assert!(
                (back.x - x).abs() < 1e-9 && (back.y - y).abs() < 1e-9 && (back.z - z).abs() < 1e-9,
                "({x}, {y}, {z}) came back as {back:?}"
            );
        }
    }

    #[test]
    fn at_the_pole_the_whole_turn_goes_to_x_and_z_is_zero_but_the_rotation_is_kept() {
        // Y = 90 degrees is the gimbal lock: X and Z turn about the same axis, so Three.js keeps only
        // one of them. The Euler triple changes; the rotation it describes must not.
        let q = Quat::from_euler_xyz_degrees(euler(20.0, 90.0, 35.0));
        let back = q.to_euler_xyz_degrees();
        assert_eq!(back.z, 0.0);
        assert!((back.y - 90.0).abs() < 1e-6);
        let rebuilt = Quat::from_euler_xyz_degrees(back);
        // Looser than elsewhere on purpose: `asin` near 1 turns a 1e-16 error in m13 into ~1e-8 rad,
        // and the angle is re-derived from it. Dropping the turn instead (a bug) would differ by O(0.1).
        assert!(rotation_of(q).max_abs_diff(&rotation_of(rebuilt)) < 1e-6);
    }
}
