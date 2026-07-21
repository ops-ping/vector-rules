//! Named geometric artifacts over embedding space: axes, calibrations, regions.
//!
//! Rules never construct geometry inline. Constructors run offline
//! ([`Axis::from_sets`], [`Region::fit`]), produce named, versioned artifacts
//! with provenance, and the GRL surface references artifacts by name
//! (`c_project(Input.text, "urgency_v1")`). Loading a ruleset validates each
//! artifact's provenance against the active embedder, so a model/dimension
//! mismatch fails at load time with a legible error instead of skewing scores.
//!
//! All membership/projection math here is a few dot products per candidate —
//! hot-path safe. Fitting is offline-tier by design.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::vec_expr::arith::{add, dot, normalize, sub};

/// Where an artifact's geometry came from. Checked at ruleset load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// Embedding model identity (name or name@sha) the vectors came from.
    pub model: String,
    /// Embedding dimensionality.
    pub dim: usize,
    /// Task prefix / prompt style used when embedding, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Identifier of the exemplar set the artifact was fitted from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exemplar_set: Option<String>,
}

/// Sorted reference scores used to convert raw projections into percentiles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    /// Ascending raw scores from the reference window.
    reference: Vec<f32>,
}

impl Calibration {
    /// Build a calibration from raw reference scores (any order).
    ///
    /// # Errors
    /// Returns `Err` if fewer than two scores are provided.
    pub fn from_scores(mut scores: Vec<f32>) -> Result<Self, String> {
        if scores.len() < 2 {
            return Err("calibration needs at least two reference scores".into());
        }
        scores.sort_by(f32::total_cmp);
        Ok(Self { reference: scores })
    }

    /// Percentile of `raw` against the reference window, in `[0, 100]`.
    #[must_use]
    pub fn percentile(&self, raw: f32) -> f32 {
        let below = self.reference.partition_point(|s| *s < raw);
        let equal = self.reference[below..]
            .iter()
            .take_while(|s| **s == raw)
            .count();
        // Midpoint convention: ties count half, so the result is symmetric.
        let rank = below as f32 + equal as f32 / 2.0;
        100.0 * rank / self.reference.len() as f32
    }
}

/// A named direction in embedding space with optional calibration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Axis {
    /// Artifact name referenced from GRL (e.g. `urgency_v1`).
    pub name: String,
    /// Fit provenance.
    pub provenance: Provenance,
    /// Unit direction vector.
    direction: Vec<f32>,
    /// Reference window for percentile scoring, if calibrated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    calibration: Option<Calibration>,
}

impl Axis {
    /// Build an axis as the normalized difference of exemplar-set centroids.
    ///
    /// Subtracting the negative centroid cancels the topic component shared by
    /// both sets and isolates the contrast direction.
    ///
    /// # Errors
    /// Returns `Err` on empty sets, mismatched dimensions, or a degenerate
    /// (near-zero) difference.
    pub fn from_sets(
        name: impl Into<String>,
        provenance: Provenance,
        positive: &[Vec<f32>],
        negative: &[Vec<f32>],
    ) -> Result<Self, String> {
        let pos = centroid(positive)?;
        let neg = centroid(negative)?;
        let diff = sub(&pos, &neg)?;
        let direction = normalize(&diff)
            .ok_or_else(|| "positive and negative centroids coincide".to_string())?;
        Ok(Self {
            name: name.into(),
            provenance,
            direction,
            calibration: None,
        })
    }

    /// Attach a percentile calibration window.
    pub fn calibrate(&mut self, calibration: Calibration) {
        self.calibration = Some(calibration);
    }

    /// Raw projection of `vector` (normalized internally) onto the axis.
    ///
    /// # Errors
    /// Returns `Err` on dimension mismatch or a zero-norm input.
    pub fn project_raw(&self, vector: &[f32]) -> Result<f32, String> {
        let unit = normalize(vector).ok_or_else(|| "cannot project zero vector".to_string())?;
        dot(&unit, &self.direction)
    }

    /// Calibrated percentile projection, if this axis carries a calibration.
    ///
    /// # Errors
    /// Returns `Err` if the axis is uncalibrated or on dimension mismatch.
    pub fn project_percentile(&self, vector: &[f32]) -> Result<f32, String> {
        let calibration = self.calibration.as_ref().ok_or_else(|| {
            format!(
                "axis `{}` has no calibration window; use the raw projection and calibrate it",
                self.name
            )
        })?;
        Ok(calibration.percentile(self.project_raw(vector)?))
    }

    /// Whether a calibration window is attached.
    #[must_use]
    pub fn is_calibrated(&self) -> bool {
        self.calibration.is_some()
    }
}

/// A low-rank ellipsoidal region fitted from an exemplar cloud.
///
/// Membership is Mahalanobis-style depth: component distances along the
/// principal directions scaled by their spread, plus the residual off-subspace
/// distance scaled by the residual spread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Region {
    /// Artifact name referenced from GRL (e.g. `bec_phrasing_v1`).
    pub name: String,
    /// Fit provenance.
    pub provenance: Provenance,
    /// Cloud centroid (of normalized exemplars).
    center: Vec<f32>,
    /// Principal unit directions (row-major, `k` of them).
    basis: Vec<Vec<f32>>,
    /// Spread (std) along each principal direction.
    scales: Vec<f32>,
    /// Spread of the residual component orthogonal to the basis.
    residual_scale: f32,
    /// Depth threshold for boolean membership, set from fit coverage.
    tau: f32,
}

impl Region {
    /// Fit a region from an exemplar cloud.
    ///
    /// `rank` principal directions are extracted by power iteration;
    /// `coverage` (e.g. `0.95`) sets the membership threshold `tau` so that
    /// that fraction of the training cloud is inside the region.
    ///
    /// # Errors
    /// Returns `Err` if the cloud is smaller than three exemplars, `rank` is
    /// zero, `coverage` is outside `(0, 1]`, or dimensions mismatch.
    pub fn fit(
        name: impl Into<String>,
        provenance: Provenance,
        cloud: &[Vec<f32>],
        rank: usize,
        coverage: f32,
    ) -> Result<Self, String> {
        if cloud.len() < 3 {
            return Err("region fit needs at least three exemplars".into());
        }
        if rank == 0 {
            return Err("region rank must be at least one".into());
        }
        if !(coverage > 0.0 && coverage <= 1.0) {
            return Err("coverage must be in (0, 1]".into());
        }
        let normalized: Vec<Vec<f32>> = cloud
            .iter()
            .map(|v| normalize(v).ok_or_else(|| "zero vector in exemplar cloud".to_string()))
            .collect::<Result<_, _>>()?;
        let center = centroid(&normalized)?;
        let centered: Vec<Vec<f32>> = normalized
            .iter()
            .map(|v| sub(v, &center))
            .collect::<Result<_, _>>()?;

        let rank = rank.min(cloud.len() - 1).min(center.len());
        let mut basis: Vec<Vec<f32>> = Vec::with_capacity(rank);
        let mut scales = Vec::with_capacity(rank);
        for seed in 0..rank {
            let Some((direction, variance)) = principal_component(&centered, &basis, seed) else {
                break;
            };
            // Directions with negligible variance carry no signal.
            let std = (variance / centered.len() as f32).sqrt();
            if std <= f32::EPSILON {
                break;
            }
            basis.push(direction);
            scales.push(std);
        }
        if basis.is_empty() {
            return Err("exemplar cloud has no measurable spread".into());
        }

        let residual_scale = {
            let mut acc = 0.0f32;
            for row in &centered {
                let residual = residual_norm_sq(row, &basis);
                acc += residual;
            }
            (acc / centered.len() as f32).sqrt().max(f32::EPSILON)
        };

        let mut region = Self {
            name: name.into(),
            provenance,
            center,
            basis,
            scales,
            residual_scale,
            tau: 0.0,
        };
        let mut depths: Vec<f32> = normalized.iter().map(|v| region.depth_of_unit(v)).collect();
        depths.sort_by(f32::total_cmp);
        let index = ((depths.len() as f32 * coverage).ceil() as usize).clamp(1, depths.len()) - 1;
        region.tau = depths[index];
        Ok(region)
    }

    /// Depth of `vector` (normalized internally): `1.0` at `tau`, smaller is
    /// deeper inside the region.
    ///
    /// # Errors
    /// Returns `Err` on dimension mismatch or a zero-norm input.
    pub fn depth(&self, vector: &[f32]) -> Result<f32, String> {
        let unit = normalize(vector).ok_or_else(|| "cannot score zero vector".to_string())?;
        if unit.len() != self.center.len() {
            return Err(format!(
                "vector dim mismatch: {} vs {}",
                unit.len(),
                self.center.len()
            ));
        }
        Ok(self.depth_of_unit(&unit) / self.tau.max(f32::EPSILON))
    }

    /// Boolean membership at the fitted coverage threshold.
    ///
    /// # Errors
    /// Returns `Err` on dimension mismatch or a zero-norm input.
    pub fn member(&self, vector: &[f32]) -> Result<bool, String> {
        Ok(self.depth(vector)? <= 1.0)
    }

    fn depth_of_unit(&self, unit: &[f32]) -> f32 {
        let centered: Vec<f32> = unit.iter().zip(&self.center).map(|(x, c)| x - c).collect();
        let mut acc = 0.0f32;
        for (direction, scale) in self.basis.iter().zip(&self.scales) {
            let component: f32 = centered.iter().zip(direction).map(|(x, d)| x * d).sum();
            acc += (component / scale.max(f32::EPSILON)).powi(2);
        }
        let residual = residual_norm_sq(&centered, &self.basis);
        acc += residual / self.residual_scale.powi(2);
        acc.sqrt()
    }
}

/// A named collection of artifacts, serializable as one JSON document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArtifactStore {
    #[serde(default)]
    axes: Vec<Axis>,
    #[serde(default)]
    regions: Vec<Region>,
}

impl ArtifactStore {
    /// Parse a store from its JSON form.
    ///
    /// # Errors
    /// Returns `Err` on malformed JSON or duplicate artifact names.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let store: Self = serde_json::from_str(json).map_err(|error| error.to_string())?;
        let mut seen = HashMap::new();
        for name in store
            .axes
            .iter()
            .map(|a| &a.name)
            .chain(store.regions.iter().map(|r| &r.name))
        {
            if seen.insert(name.clone(), ()).is_some() {
                return Err(format!("duplicate artifact name `{name}`"));
            }
        }
        Ok(store)
    }

    /// Serialize the store to JSON.
    ///
    /// # Errors
    /// Returns `Err` if serialization fails.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|error| error.to_string())
    }

    /// Add an axis, replacing any artifact with the same name.
    pub fn insert_axis(&mut self, axis: Axis) {
        self.axes.retain(|existing| existing.name != axis.name);
        self.axes.push(axis);
    }

    /// Add a region, replacing any artifact with the same name.
    pub fn insert_region(&mut self, region: Region) {
        self.regions.retain(|existing| existing.name != region.name);
        self.regions.push(region);
    }

    /// Look up an axis by name.
    #[must_use]
    pub fn axis(&self, name: &str) -> Option<&Axis> {
        self.axes.iter().find(|a| a.name == name)
    }

    /// Look up a region by name.
    #[must_use]
    pub fn region(&self, name: &str) -> Option<&Region> {
        self.regions.iter().find(|r| r.name == name)
    }

    /// `true` when the store holds no artifacts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.axes.is_empty() && self.regions.is_empty()
    }

    /// Validate every artifact against the active embedder identity.
    ///
    /// # Errors
    /// Returns `Err` naming the first artifact whose provenance does not match.
    pub fn validate_provenance(&self, model: &str, dim: usize) -> Result<(), String> {
        for (name, provenance) in self
            .axes
            .iter()
            .map(|a| (&a.name, &a.provenance))
            .chain(self.regions.iter().map(|r| (&r.name, &r.provenance)))
        {
            if provenance.dim != dim {
                return Err(format!(
                    "artifact `{name}` was fitted at dim {} but the active embedder emits dim {dim}",
                    provenance.dim
                ));
            }
            if provenance.model != model {
                return Err(format!(
                    "artifact `{name}` was fitted against model `{}` but the active embedder is `{model}`",
                    provenance.model
                ));
            }
        }
        Ok(())
    }
}

fn centroid(vectors: &[Vec<f32>]) -> Result<Vec<f32>, String> {
    let first = vectors
        .first()
        .ok_or_else(|| "empty exemplar set".to_string())?;
    let mut acc = vec![0.0f32; first.len()];
    for vector in vectors {
        acc = add(&acc, vector)?;
    }
    let n = vectors.len() as f32;
    for a in &mut acc {
        *a /= n;
    }
    Ok(acc)
}

/// Leading principal component of `centered` rows orthogonal to `existing`,
/// via power iteration on the implicit covariance. Returns the unit direction
/// and its eigenvalue (sum of squared projections).
fn principal_component(
    centered: &[Vec<f32>],
    existing: &[Vec<f32>],
    seed: usize,
) -> Option<(Vec<f32>, f32)> {
    let dim = centered.first()?.len();
    // Deterministic pseudo-random start so fits are reproducible.
    let mut v: Vec<f32> = (0..dim)
        .map(|i| {
            let x = ((i + 1) * (seed + 3)) as f32;
            (x.sin() * 43_758.547).fract()
        })
        .collect();
    orthogonalize(&mut v, existing);
    v = normalize(&v)?;
    let mut eigenvalue = 0.0f32;
    for _ in 0..64 {
        // w = Σ_i (x_i · v) x_i  — covariance matvec without the d×d matrix.
        let mut w = vec![0.0f32; dim];
        for row in centered {
            let coefficient: f32 = row.iter().zip(&v).map(|(x, y)| x * y).sum();
            for (wi, xi) in w.iter_mut().zip(row) {
                *wi += coefficient * xi;
            }
        }
        orthogonalize(&mut w, existing);
        eigenvalue = w.iter().map(|x| x * x).sum::<f32>().sqrt();
        match normalize(&w) {
            Some(next) => v = next,
            None => return None,
        }
    }
    Some((v, eigenvalue))
}

fn orthogonalize(v: &mut [f32], basis: &[Vec<f32>]) {
    for direction in basis {
        let component: f32 = v.iter().zip(direction).map(|(x, d)| x * d).sum();
        for (vi, di) in v.iter_mut().zip(direction) {
            *vi -= component * di;
        }
    }
}

fn residual_norm_sq(centered: &[f32], basis: &[Vec<f32>]) -> f32 {
    let mut residual = centered.to_vec();
    orthogonalize(&mut residual, basis);
    residual.iter().map(|x| x * x).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prov() -> Provenance {
        Provenance {
            model: "test-model".into(),
            dim: 3,
            task: None,
            exemplar_set: Some("set_v1".into()),
        }
    }

    #[test]
    fn axis_isolates_contrast_direction() {
        // Positive and negative sets share dim0 (topic); they differ on dim1.
        let pos = vec![vec![1.0, 1.0, 0.0], vec![1.0, 0.8, 0.1]];
        let neg = vec![vec![1.0, -1.0, 0.0], vec![1.0, -0.8, -0.1]];
        let axis = Axis::from_sets("polarity", prov(), &pos, &neg).unwrap();
        let projected = axis.project_raw(&[1.0, 1.0, 0.0]).unwrap();
        let anti = axis.project_raw(&[1.0, -1.0, 0.0]).unwrap();
        assert!(projected > 0.0 && anti < 0.0, "{projected} vs {anti}");
    }

    #[test]
    fn degenerate_axis_is_rejected() {
        let same = vec![vec![1.0, 0.0, 0.0]];
        assert!(Axis::from_sets("null", prov(), &same, &same).is_err());
    }

    #[test]
    fn percentile_is_monotonic_and_bounded() {
        let calibration = Calibration::from_scores(vec![0.1, 0.2, 0.3, 0.4]).unwrap();
        assert_eq!(calibration.percentile(0.05), 0.0);
        assert_eq!(calibration.percentile(0.9), 100.0);
        let mid = calibration.percentile(0.25);
        assert!(mid > 25.0 && mid < 75.0, "{mid}");
        assert!(calibration.percentile(0.35) > mid);
    }

    #[test]
    fn calibrated_projection_requires_calibration() {
        let pos = vec![vec![1.0, 1.0, 0.0]];
        let neg = vec![vec![1.0, -1.0, 0.0]];
        let mut axis = Axis::from_sets("polarity", prov(), &pos, &neg).unwrap();
        assert!(axis.project_percentile(&[1.0, 0.5, 0.0]).is_err());
        axis.calibrate(Calibration::from_scores(vec![-0.5, 0.0, 0.5]).unwrap());
        let pct = axis.project_percentile(&[1.0, 1.0, 0.0]).unwrap();
        assert!((0.0..=100.0).contains(&pct));
    }

    #[test]
    fn region_membership_covers_cloud_and_rejects_outliers() {
        // Tight cloud near (1, 0, 0) with small independent spread in dims 1-2.
        let cloud: Vec<Vec<f32>> = (0..12)
            .map(|i| {
                let wobble = ((i % 4) as f32 - 1.5) / 50.0;
                let jitter = (((i * 7) % 5) as f32 - 2.0) / 100.0;
                vec![1.0, wobble, jitter]
            })
            .collect();
        let region = Region::fit("tight", prov(), &cloud, 2, 0.95).unwrap();
        let inside = region.member(&[1.0, 0.02, 0.0]).unwrap();
        let outside = region.member(&[0.0, 1.0, 0.0]).unwrap();
        assert!(inside);
        assert!(!outside);
        let deep = region.depth(&[1.0, 0.0, 0.0]).unwrap();
        let far = region.depth(&[0.0, 1.0, 0.0]).unwrap();
        assert!(deep < far);
    }

    #[test]
    fn store_round_trips_and_validates_provenance() {
        let pos = vec![vec![1.0, 1.0, 0.0]];
        let neg = vec![vec![1.0, -1.0, 0.0]];
        let mut store = ArtifactStore::default();
        store.insert_axis(Axis::from_sets("polarity", prov(), &pos, &neg).unwrap());
        let json = store.to_json().unwrap();
        let restored = ArtifactStore::from_json(&json).unwrap();
        assert!(restored.axis("polarity").is_some());
        assert!(restored.validate_provenance("test-model", 3).is_ok());
        assert!(restored.validate_provenance("test-model", 4).is_err());
        assert!(restored.validate_provenance("other-model", 3).is_err());
    }

    #[test]
    fn duplicate_artifact_names_are_rejected() {
        let pos = vec![vec![1.0, 1.0, 0.0]];
        let neg = vec![vec![1.0, -1.0, 0.0]];
        let axis = Axis::from_sets("dup", prov(), &pos, &neg).unwrap();
        let mut store = ArtifactStore::default();
        store.insert_axis(axis.clone());
        let mut json: serde_json::Value = serde_json::from_str(&store.to_json().unwrap()).unwrap();
        let duplicate = json["axes"][0].clone();
        json["axes"].as_array_mut().unwrap().push(duplicate);
        assert!(ArtifactStore::from_json(&json.to_string()).is_err());
    }
}
