//! Spline meshes (USplineMeshComponent): a static mesh bent along a cubic Hermite segment, as Unreal's vertex
//! factory does it (LocalVertexFactory.ush's CalcSliceTransform / CalcSliceRot under USE_SPLINEDEFORM, UE 4.18; the
//! engine's published shader, recalled, not read from the game: MODLOG "Spline meshes"). The landscape's roads and
//! the guard rails are spline meshes. Everything here is in the component's local space, in Unreal units (cm, Z up).

/// A component's spline (its SplineParams, SplineUpDir, ForwardAxis, SplineBoundaryMin/Max,
/// bSmoothInterpRollScale).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplineParams {
    pub start_pos: [f32; 3],
    pub start_tangent: [f32; 3],
    pub start_scale: [f32; 2],
    /// Radians.
    pub start_roll: f32,
    pub start_offset: [f32; 2],
    pub end_pos: [f32; 3],
    pub end_tangent: [f32; 3],
    pub end_scale: [f32; 2],
    pub end_roll: f32,
    pub end_offset: [f32; 2],
    pub up: [f32; 3],
    /// 0 X, 1 Y, 2 Z.
    pub forward_axis: usize,
    /// The mesh's extent along the forward axis the spline spans (SplineBoundaryMin/Max when set, else the mesh's
    /// bounds).
    pub boundary: (f32, f32),
    pub smooth_interp_roll_scale: bool,
}

type V = [f32; 3];

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: V, s: f32) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn normalize(a: V) -> V {
    let l = dot(a, a).sqrt();
    if l > 1e-12 {
        mul(a, 1.0 / l)
    } else {
        [0.0; 3]
    }
}
fn lerp2(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

impl SplineParams {
    /// The mesh's axes: (SplineMeshDir, SplineMeshX, SplineMeshY) for the forward axis.
    pub fn mesh_axes(&self) -> (V, V, V) {
        match self.forward_axis {
            1 => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
            2 => ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            _ => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        }
    }

    /// SplineEvalPos: the Hermite curve at `a`.
    pub fn pos(&self, a: f32) -> V {
        let (a2, a3) = (a * a, a * a * a);
        add(
            add(mul(self.start_pos, 2.0 * a3 - 3.0 * a2 + 1.0), mul(self.start_tangent, a3 - 2.0 * a2 + a)),
            add(mul(self.end_tangent, a3 - a2), mul(self.end_pos, -2.0 * a3 + 3.0 * a2)),
        )
    }

    /// SplineEvalDir: the curve's normalized derivative at `a`.
    pub fn dir(&self, a: f32) -> V {
        let a2 = a * a;
        normalize(add(
            add(mul(self.start_pos, 6.0 * a2 - 6.0 * a), mul(self.start_tangent, 3.0 * a2 - 4.0 * a + 1.0)),
            add(mul(self.end_tangent, 3.0 * a2 - 2.0 * a), mul(self.end_pos, -6.0 * a2 + 6.0 * a)),
        ))
    }

    /// The slice at a distance `z` along the forward axis: its point (offset applied) and its (spline direction,
    /// X, Y) frame, X and Y rolled and unscaled, and the scale.
    fn slice(&self, z: f32) -> (V, V, V, V, [f32; 2]) {
        let (min, max) = self.boundary;
        let a = (z - min) / (max - min).max(1e-6);
        let h = if self.smooth_interp_roll_scale { a * a * (3.0 - 2.0 * a) } else { a };
        let mut p = self.pos(a);
        let d = self.dir(a);
        let bx = normalize(cross(self.up, d));
        let by = normalize(cross(d, bx));
        let off = lerp2(self.start_offset, self.end_offset, h);
        p = add(p, add(mul(bx, off[0]), mul(by, off[1])));
        let roll = self.start_roll + (self.end_roll - self.start_roll) * h;
        let (s, c) = roll.sin_cos();
        let x = add(mul(bx, c), mul(by, -s));
        let y = add(mul(by, c), mul(bx, s));
        (p, d, x, y, lerp2(self.start_scale, self.end_scale, h))
    }

    /// A vertex (and its normal) of the mesh, bent: the slice at its forward coordinate, its other two coordinates
    /// along the slice's scaled X and Y (CalcSliceTransform); the normal turned by the slice's unscaled frame
    /// (CalcSliceRot).
    pub fn deform(&self, p: V, n: V) -> (V, V) {
        let (md, mx, my) = self.mesh_axes();
        let (sp, d, x, y, scale) = self.slice(dot(p, md));
        let pos = add(sp, add(mul(x, dot(p, mx) * scale[0]), mul(y, dot(p, my) * scale[1])));
        let normal = normalize(add(mul(d, dot(n, md)), add(mul(x, dot(n, mx)), mul(y, dot(n, my)))));
        (pos, normal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight() -> SplineParams {
        SplineParams {
            start_pos: [0.0, 0.0, 0.0],
            start_tangent: [100.0, 0.0, 0.0],
            start_scale: [1.0, 1.0],
            start_roll: 0.0,
            start_offset: [0.0, 0.0],
            end_pos: [100.0, 0.0, 0.0],
            end_tangent: [100.0, 0.0, 0.0],
            end_scale: [1.0, 1.0],
            end_roll: 0.0,
            end_offset: [0.0, 0.0],
            up: [0.0, 0.0, 1.0],
            forward_axis: 0,
            boundary: (0.0, 100.0),
            smooth_interp_roll_scale: false,
        }
    }

    fn close(a: V, b: V) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3)
    }

    #[test]
    fn a_straight_spline_matching_the_mesh_leaves_it_as_it_is() {
        let s = straight();
        for p in [[0.0, 0.0, 0.0], [50.0, 10.0, 5.0], [100.0, -20.0, 3.0]] {
            let (q, n) = s.deform(p, [0.0, 0.0, 1.0]);
            assert!(close(q, p), "{p:?} -> {q:?}");
            assert!(close(n, [0.0, 0.0, 1.0]));
        }
    }

    #[test]
    fn the_mesh_ends_land_on_the_spline_ends() {
        let mut s = straight();
        s.end_pos = [200.0, 50.0, 30.0];
        s.end_tangent = [200.0, 100.0, 0.0];
        // The mesh's centre line, start and end.
        assert!(close(s.deform([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]).0, s.start_pos));
        assert!(close(s.deform([100.0, 0.0, 0.0], [0.0, 0.0, 1.0]).0, s.end_pos));
    }

    #[test]
    fn width_follows_the_spline_side_and_scale() {
        let mut s = straight();
        s.start_scale = [2.0, 1.0];
        s.end_scale = [2.0, 1.0];
        // The mesh's +Y side (X forward: SplineMeshX = Y) goes along cross(up, dir) = +Y, doubled.
        assert!(close(s.deform([0.0, 10.0, 0.0], [0.0, 0.0, 1.0]).0, [0.0, 20.0, 0.0]));
    }
}
