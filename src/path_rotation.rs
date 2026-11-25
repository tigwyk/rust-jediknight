/// State for the non-pivot path orientation branch. Angles are pitch, yaw, roll in degrees.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PathRotation {
    /// Engine row-vector orientation at the start of the segment.
    pub(crate) start_matrix: [[f32; 3]; 3],
    /// Current engine row-vector orientation.
    pub(crate) matrix: [[f32; 3]; 3],
    pub(crate) delta: [f32; 3],
    pub(crate) target: [f32; 3],
    pub(crate) rate: f32,
    pub(crate) progress: f32,
    pub(crate) active: bool,
}

impl PathRotation {
    /// Prepare a segment from the current and target Euler angles and duration in seconds.
    pub(crate) fn prepare(current: [f32; 3], target: [f32; 3], duration: f32) -> Self {
        let matrix = build_rotation(current);
        let extracted = extract_angles(matrix);
        let mut delta = [0.0; 3];
        for i in 0..3 {
            let angle = (target[i] - extracted[i]).rem_euclid(360.0);
            delta[i] = if angle > 180.0 { angle - 360.0 } else { angle };
            if delta[i].abs() < 0.001 {
                delta[i] = 0.0;
            }
        }
        let active = delta != [0.0; 3];
        Self { start_matrix: matrix, matrix, delta, target, rate: if active { 1.0 / duration } else { 0.0 }, progress: 0.0, active }
    }

    /// Advance by seconds. Completion snaps to the target matrix, as the engine does.
    pub(crate) fn advance(&mut self, dt: f32) {
        if !self.active || dt == 0.0 {
            return;
        }
        self.progress += self.rate * dt;
        if self.progress > 1.0 {
            self.progress = 1.0;
        }
        let rotation = build_rotation(self.delta.map(|angle| angle * self.progress));
        self.matrix = multiply(self.start_matrix, rotation);
        if self.progress >= 1.0 {
            self.matrix = build_rotation(self.target);
            self.active = false;
        }
    }

    pub(crate) fn current_euler(&self) -> [f32; 3] {
        extract_angles(self.matrix)
    }
}

fn build_rotation([pitch, yaw, roll]: [f32; 3]) -> [[f32; 3]; 3] {
    let (sx, cx) = pitch.to_radians().sin_cos();
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (sz, cz) = roll.to_radians().sin_cos();
    [
        [-(sz * sy) * sx + cz * cy, sz * cy * sx + cz * sy, -sz * cx],
        [-sy * cx, cy * cx, sx],
        [cz * sy * sx + sz * cy, -sx * cz * cy + sz * sy, cz * cx],
    ]
}

fn multiply(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut result = [[0.0; 3]; 3];
    for row in 0..3 {
        for col in 0..3 {
            result[row][col] = a[row][0] * b[0][col] + a[row][1] * b[1][col] + a[row][2] * b[2][col];
        }
    }
    result
}

fn extract_angles(m: [[f32; 3]; 3]) -> [f32; 3] {
    let x = m[1][0];
    let y = m[1][1];
    let z = m[1][2];
    let length_squared = y * y + x * x;
    let length = length_squared.sqrt();
    let mut angles = [0.0; 3];
    if length >= 0.000001 {
        angles[1] = 90.0 - (y / length).clamp(-1.0, 1.0).asin().to_degrees();
        if x > 0.0 { angles[1] = -angles[1]; }
        let cos_pitch = length_squared / length;
        if cos_pitch < 1.0 {
            angles[0] = 90.0 - cos_pitch.asin().to_degrees();
        }
        let cos_roll = (-m[0][1] * x + y * m[0][0]) / length;
        angles[2] = if cos_roll >= 1.0 { 0.0 } else if cos_roll <= -1.0 { 180.0 } else { 90.0 - cos_roll.asin().to_degrees() };
        if m[0][2] > 0.0 { angles[2] = -angles[2]; }
    } else {
        angles[0] = 90.0;
        angles[2] = 90.0 - m[0][0].clamp(-1.0, 1.0).asin().to_degrees();
        if (-m[0][1] > 0.0 && z > 0.0) || (-m[0][1] < 0.0 && z < 0.0) {
            angles[2] = -angles[2];
        }
    }
    if z < 0.0 { angles[0] = -angles[0]; }
    angles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_composes_and_snaps_to_target() {
        let mut rotation = PathRotation::prepare([20.0, 30.0, 10.0], [50.0, 70.0, -20.0], 2.0);
        rotation.advance(1.0);
        assert!(rotation.active);
        assert_eq!(rotation.progress, 0.5);
        assert_ne!(rotation.matrix, build_rotation([35.0, 50.0, -5.0]));
        rotation.advance(1.0);
        assert!(!rotation.active);
        assert_eq!(rotation.matrix, build_rotation(rotation.target));
    }
}
