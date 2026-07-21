//! Pure f32 vector arithmetic shared by the GRL functions and geometry.
//!
//! Dependency-free, `#[inline]` slice ops that LLVM auto-vectorizes; they have
//! no knowledge of embeddings or the engine, so they unit-test without any
//! model.

/// Elementwise sum `a + b`.
///
/// # Errors
/// Returns `Err` if the operands have different lengths (a dimension mismatch
/// means the two embeddings came from incompatible models).
#[inline]
pub fn add(a: &[f32], b: &[f32]) -> Result<Vec<f32>, String> {
    if a.len() != b.len() {
        return Err(format!("vector dim mismatch: {} vs {}", a.len(), b.len()));
    }
    Ok(a.iter().zip(b).map(|(x, y)| x + y).collect())
}

/// Elementwise difference `a - b`.
///
/// # Errors
/// Returns `Err` on a dimension mismatch.
#[inline]
pub fn sub(a: &[f32], b: &[f32]) -> Result<Vec<f32>, String> {
    if a.len() != b.len() {
        return Err(format!("vector dim mismatch: {} vs {}", a.len(), b.len()));
    }
    Ok(a.iter().zip(b).map(|(x, y)| x - y).collect())
}

/// Dot product of `a` and `b`.
///
/// # Errors
/// Returns `Err` on a dimension mismatch.
#[inline]
pub fn dot(a: &[f32], b: &[f32]) -> Result<f32, String> {
    if a.len() != b.len() {
        return Err(format!("vector dim mismatch: {} vs {}", a.len(), b.len()));
    }
    Ok(a.iter().zip(b).map(|(x, y)| x * y).sum())
}

/// Unit-normalized copy of `v`, or `None` for a zero-norm vector.
#[inline]
#[must_use]
pub fn normalize(v: &[f32]) -> Option<Vec<f32>> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 || !norm.is_finite() {
        return None;
    }
    Some(v.iter().map(|x| x / norm).collect())
}

/// Cosine similarity of `a` and `b`, in `[-1.0, 1.0]`.
///
/// A zero-norm operand yields `0.0` (no NaN), so empty/degenerate inputs are
/// deterministic.
///
/// # Errors
/// Returns `Err` on a dimension mismatch.
#[inline]
pub fn cosine_sim(a: &[f32], b: &[f32]) -> Result<f32, String> {
    if a.len() != b.len() {
        return Err(format!("vector dim mismatch: {} vs {}", a.len(), b.len()));
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 {
        return Ok(0.0);
    }
    Ok(dot / denom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_sub_are_elementwise() {
        assert_eq!(add(&[1.0, 2.0], &[3.0, 4.0]).unwrap(), vec![4.0, 6.0]);
        assert_eq!(sub(&[3.0, 4.0], &[1.0, 2.0]).unwrap(), vec![2.0, 2.0]);
    }

    #[test]
    fn dim_mismatch_is_error() {
        assert!(add(&[1.0], &[1.0, 2.0]).is_err());
        assert!(sub(&[1.0], &[1.0, 2.0]).is_err());
        assert!(cosine_sim(&[1.0], &[1.0, 2.0]).is_err());
    }

    #[test]
    fn cosine_identical_is_one() {
        let v = [0.3, -0.4, 0.5];
        let c = cosine_sim(&v, &v).unwrap();
        assert!((c - 1.0).abs() < 1e-6, "got {c}");
    }

    #[test]
    fn cosine_orthogonal_is_zero() {
        let c = cosine_sim(&[1.0, 0.0], &[0.0, 1.0]).unwrap();
        assert!(c.abs() < 1e-6, "got {c}");
    }

    #[test]
    fn cosine_opposite_is_negative_one() {
        let c = cosine_sim(&[1.0, 0.0], &[-1.0, 0.0]).unwrap();
        assert!((c + 1.0).abs() < 1e-6, "got {c}");
    }

    #[test]
    fn cosine_zero_vector_is_zero_not_nan() {
        let c = cosine_sim(&[0.0, 0.0], &[1.0, 2.0]).unwrap();
        assert_eq!(c, 0.0);
        assert!(!c.is_nan());
    }

    #[test]
    fn analogy_arithmetic_composes() {
        // (king - man) + woman, with toy orthogonal axes:
        // dim0 = royalty, dim1 = gender (male +1 / female -1).
        let king = [1.0, 1.0];
        let man = [0.0, 1.0];
        let woman = [0.0, -1.0];
        let queen = [1.0, -1.0];
        let delta = sub(&king, &man).unwrap(); // [1, 0]
        let analogy = add(&delta, &woman).unwrap(); // [1, -1]
        let c = cosine_sim(&analogy, &queen).unwrap();
        assert!(
            (c - 1.0).abs() < 1e-6,
            "analogy should equal queen, got {c}"
        );
    }
}
