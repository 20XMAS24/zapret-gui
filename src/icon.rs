//! App icon drawn in code (no resource compiler needed): gradient disc with a white "Z".

fn seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

/// RGBA pixels of a `size`×`size` icon. `dot` adds a small status dot in the corner.
pub fn rgba(size: u32, dot: Option<[u8; 3]>) -> Vec<u8> {
    let n = size as f32;
    let mut v = vec![0u8; (size * size * 4) as usize];
    let c = n / 2.0;
    let r = n / 2.0 - 0.5;
    let segs = [
        ((0.31, 0.31), (0.69, 0.31)),
        ((0.69, 0.31), (0.31, 0.69)),
        ((0.31, 0.69), (0.69, 0.69)),
    ];
    let half = n * 0.075;
    let (dot_c, dot_r) = ((n * 0.80, n * 0.80), n * 0.17);
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let d = ((px - c).powi(2) + (py - c).powi(2)).sqrt();
            let mut alpha = (r - d + 0.5).clamp(0.0, 1.0);
            let t = ((px + py) / (2.0 * n)).clamp(0.0, 1.0);
            let mut col = [
                0x6d as f32 + (0x8b as f32 - 0x6d as f32) * t,
                0x8d as f32 + (0x5c as f32 - 0x8d as f32) * t,
                0xff as f32 + (0xf6 as f32 - 0xff as f32) * t,
            ];
            let dz = segs
                .iter()
                .map(|((ax, ay), (bx, by))| seg_dist(px, py, ax * n, ay * n, bx * n, by * n))
                .fold(f32::MAX, f32::min);
            let az = (half - dz + 0.5).clamp(0.0, 1.0);
            for ch in col.iter_mut() {
                *ch += (255.0 - *ch) * az;
            }
            if let Some(dc) = dot {
                let dd = ((px - dot_c.0).powi(2) + (py - dot_c.1).powi(2)).sqrt();
                let ring = (dot_r + n * 0.05 - dd + 0.5).clamp(0.0, 1.0);
                let fill = (dot_r - dd + 0.5).clamp(0.0, 1.0);
                if ring > 0.0 {
                    // dark ring separates the dot from the disc
                    for ch in col.iter_mut() {
                        *ch *= 1.0 - ring;
                    }
                    col[0] += 0x14 as f32 * ring;
                    col[1] += 0x17 as f32 * ring;
                    col[2] += 0x20 as f32 * ring;
                    for (i, ch) in col.iter_mut().enumerate() {
                        *ch += (dc[i] as f32 - *ch) * fill;
                    }
                    alpha = alpha.max(ring);
                }
            }
            let i = ((y * size + x) * 4) as usize;
            v[i] = col[0].round() as u8;
            v[i + 1] = col[1].round() as u8;
            v[i + 2] = col[2].round() as u8;
            v[i + 3] = (alpha * 255.0).round() as u8;
        }
    }
    v
}
