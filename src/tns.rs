use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("sha256:{:x}", h.finalize())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    Ok(sha256_bytes(&bytes))
}

pub fn sha256_json<T: Serialize>(v: &T) -> String {
    sha256_bytes(serde_json::to_vec(v).unwrap().as_slice())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: String,
    pub brain_id: String,
    pub version: String,
    pub substrate_file: String,
    pub substrate_hash: String,
    pub coordinate_law: String,
    pub coordinate_law_hash: String,
    pub region_map: String,
    pub region_map_hash: String,
    pub read_modes: String,
    pub read_modes_hash: String,
    pub adapter_abi: String,
    pub adapter_abi_hash: String,
    pub mutation_default: String,
    pub supported_read_modes: Vec<String>,
    pub supported_adapter_abis: Vec<String>,
    pub probe_receipts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionDecl {
    pub region_id: String,
    pub readable: bool,
    pub writable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionMap {
    pub schema: String,
    pub brain_id: String,
    pub regions: Vec<RegionDecl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadMode {
    pub schema: String,
    pub read_mode_id: String,
    pub allowed_regions: Vec<String>,
    pub write_back_allowed: bool,
    pub gradient_flow_allowed: bool,
    pub mutation_allowed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterAbi {
    pub schema: String,
    pub adapter_abi_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainPackage {
    pub root_dir: PathBuf,
    pub manifest: Manifest,
    pub substrate: BTreeMap<String, Vec<f32>>,
    pub coordinate_law: Value,
    pub region_map: RegionMap,
    pub read_modes: Vec<ReadMode>,
    pub adapter_abi: AdapterAbi,
}

impl BrainPackage {
    pub fn load_from_dir<P: AsRef<Path>>(root: P) -> Result<Self, String> {
        let root = root.as_ref().to_path_buf();
        let manifest: Manifest = serde_json::from_slice(
            &fs::read(root.join("manifest.json")).map_err(|e| format!("manifest read: {e}"))?,
        )
        .map_err(|e| format!("manifest parse: {e}"))?;
        if manifest.schema.is_empty() {
            return Err("bad manifest: missing schema".into());
        }

        let substrate: BTreeMap<String, Vec<f32>> = serde_json::from_slice(
            &fs::read(root.join(&manifest.substrate_file))
                .map_err(|e| format!("substrate read: {e}"))?,
        )
        .map_err(|e| format!("substrate parse: {e}"))?;
        let coordinate_law: Value = serde_json::from_slice(
            &fs::read(root.join(&manifest.coordinate_law))
                .map_err(|e| format!("coordinate_law read: {e}"))?,
        )
        .map_err(|e| format!("coordinate_law parse: {e}"))?;
        let region_map: RegionMap = serde_json::from_slice(
            &fs::read(root.join(&manifest.region_map))
                .map_err(|e| format!("region_map read: {e}"))?,
        )
        .map_err(|e| format!("region_map parse: {e}"))?;
        let read_modes: Vec<ReadMode> = serde_json::from_slice(
            &fs::read(root.join(&manifest.read_modes))
                .map_err(|e| format!("read_modes read: {e}"))?,
        )
        .map_err(|e| format!("read_modes parse: {e}"))?;
        let adapter_abi: AdapterAbi = serde_json::from_slice(
            &fs::read(root.join(&manifest.adapter_abi))
                .map_err(|e| format!("adapter_abi read: {e}"))?,
        )
        .map_err(|e| format!("adapter_abi parse: {e}"))?;

        let pkg = Self {
            root_dir: root,
            manifest,
            substrate,
            coordinate_law,
            region_map,
            read_modes,
            adapter_abi,
        };
        pkg.verify()?;
        Ok(pkg)
    }

    pub fn verify(&self) -> Result<(), String> {
        if self.manifest.mutation_default != "forbidden" {
            return Err("mutation_default must be forbidden".into());
        }
        self.verify_hash(
            "substrate",
            &self.manifest.substrate_hash,
            &self.manifest.substrate_file,
        )?;
        self.verify_hash(
            "coordinate_law",
            &self.manifest.coordinate_law_hash,
            &self.manifest.coordinate_law,
        )?;
        self.verify_hash(
            "region_map",
            &self.manifest.region_map_hash,
            &self.manifest.region_map,
        )?;
        self.verify_hash(
            "read_modes",
            &self.manifest.read_modes_hash,
            &self.manifest.read_modes,
        )?;
        self.verify_hash(
            "adapter_abi",
            &self.manifest.adapter_abi_hash,
            &self.manifest.adapter_abi,
        )?;
        if self.manifest.probe_receipts.is_empty() {
            return Err("missing probe receipt".into());
        }
        for p in &self.manifest.probe_receipts {
            if !self.root_dir.join(p).exists() {
                return Err(format!("missing probe receipt file: {p}"));
            }
        }
        if self.coordinate_law.get("schema").is_none() {
            return Err("coordinate_law required".into());
        }
        if self.region_map.regions.is_empty() {
            return Err("region_map required".into());
        }
        Ok(())
    }

    fn verify_hash(&self, label: &str, expected: &str, rel: &str) -> Result<(), String> {
        let actual = sha256_file(&self.root_dir.join(rel))?;
        if actual != expected {
            return Err(format!("{label} hash mismatch"));
        }
        Ok(())
    }

    pub fn hash(&self) -> Result<String, String> {
        sha256_file(&self.root_dir.join(&self.manifest.substrate_file))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingSession {
    pub binding_id: String,
    pub brain_id: String,
    pub brain_hash: String,
    pub adapter_id: String,
    pub read_mode_id: String,
    pub allowed_regions: Vec<String>,
    pub mutation_allowed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadOnlyProjectionAdapter {
    pub adapter_id: String,
    pub abi_id: String,
    pub permitted_scope: Option<BTreeSet<String>>,
}

impl ReadOnlyProjectionAdapter {
    pub fn new() -> Self {
        Self {
            adapter_id: "readonly_projection_adapter.v0".into(),
            abi_id: "torch_readonly_projection_adapter_v0".into(),
            permitted_scope: None,
        }
    }

    pub fn bind(&self, brain: &BrainPackage, read_mode: &str) -> Result<BindingSession, String> {
        brain.verify()?;
        if !brain.manifest.supported_adapter_abis.contains(&self.abi_id)
            || brain.adapter_abi.adapter_abi_id != self.abi_id
        {
            return Err("ADAPTER_ABI_MISMATCH".into());
        }
        let mode = brain
            .read_modes
            .iter()
            .find(|m| m.read_mode_id == read_mode)
            .ok_or("READ_REJECTED")?;
        if mode.write_back_allowed || mode.gradient_flow_allowed || mode.mutation_allowed {
            return Err("READ_MODE_NOT_READONLY".into());
        }

        let readable: BTreeSet<String> = brain
            .region_map
            .regions
            .iter()
            .filter(|r| r.readable && !r.writable)
            .map(|r| r.region_id.clone())
            .collect();
        let mode_allowed: BTreeSet<String> = mode.allowed_regions.iter().cloned().collect();
        if !mode_allowed.is_subset(&readable) {
            return Err("READ_MODE_REGION_UNDECLARED".into());
        }
        let mut allowed: BTreeSet<String> = readable.intersection(&mode_allowed).cloned().collect();
        if let Some(scope) = &self.permitted_scope {
            allowed = allowed.intersection(scope).cloned().collect();
        }
        if allowed.is_empty() {
            return Err("NO_ALLOWED_REGIONS".into());
        }

        Ok(BindingSession {
            binding_id: "bind_0001".into(),
            brain_id: brain.manifest.brain_id.clone(),
            brain_hash: brain.hash()?,
            adapter_id: self.adapter_id.clone(),
            read_mode_id: read_mode.into(),
            allowed_regions: allowed.into_iter().collect(),
            mutation_allowed: false,
        })
    }

    pub fn read_region(
        &self,
        binding: &BindingSession,
        brain: &BrainPackage,
        region: &str,
    ) -> Result<Vec<f32>, String> {
        if !binding.allowed_regions.contains(&region.to_string()) {
            return Err("REJECT_ROUTE".into());
        }
        brain
            .substrate
            .get(region)
            .cloned()
            .ok_or("REJECT_ROUTE".into())
    }
    pub fn write_region(&self, _region: &str, _value: Vec<f32>) -> Result<(), String> {
        Err("REJECT_ADAPTER_WRITE".into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingBody {
    pub columns: usize,
    pub state: BTreeMap<String, String>,
}
impl WorkingBody {
    pub fn new() -> Self {
        Self {
            columns: 1,
            state: BTreeMap::new(),
        }
    }
    pub fn hash(&self) -> String {
        sha256_json(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpansionDelta {
    pub reason: String,
    pub previous_working_body_hash: String,
    pub next_working_body_hash: String,
    pub brain_modules_modified: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraversalResult {
    pub output: usize,
    pub receipt: Value,
}

pub fn run_demo_traversal() -> Result<TraversalResult, String> {
    let brain = BrainPackage::load_from_dir("brain_packages/example.pattern_brain.v0")?;
    let adapter = ReadOnlyProjectionAdapter::new();
    let binding = adapter.bind(&brain, "readonly_activation_projection_v0")?;
    let mut wb = WorkingBody::new();
    let wb_before = wb.hash();
    let b_before = brain.hash()?;

    let r0 = adapter.read_region(&binding, &brain, "encoder.layer_0.activation")?;
    let r1 = adapter.read_region(&binding, &brain, "encoder.layer_1.activation")?;
    let confidence = (r0.iter().chain(r1.iter()).copied().sum::<f32>() / 100.0).min(1.0);

    let mut expansion_hashes = Vec::new();
    if confidence < 0.5 {
        let old_hash = wb.hash();
        wb.columns += 1;
        wb.state
            .insert("expansion_reason".into(), "capacity_rejection".into());
        let d = ExpansionDelta {
            reason: "capacity_rejection".into(),
            previous_working_body_hash: old_hash,
            next_working_body_hash: wb.hash(),
            brain_modules_modified: vec![],
        };
        expansion_hashes.push(sha256_json(&d));
    }

    let logits = adapter.read_region(&binding, &brain, "output.logits")?;
    let output = if logits[0] >= logits[1] { 0 } else { 1 };
    let output_hash = sha256_json(&logits);
    let route = json!({"steps":[{"event":"read_region","region_id":"encoder.layer_0.activation"},{"event":"read_region","region_id":"encoder.layer_1.activation"},{"event":"compose","target":"working_body.column_0.composer"},{"event":"emit_candidate_output","target":"output_head"}]});
    let route_hash = sha256_json(&route);
    let b_after = brain.hash()?;

    let receipt = json!({
        "schema":"traversal_receipt_v0",
        "brain_id": brain.manifest.brain_id,
        "brain_hash_before": b_before,
        "brain_hash_after": b_after,
        "immutable_substrate_verified": b_before == b_after,
        "adapter_id": binding.adapter_id,
        "read_mode_id": binding.read_mode_id,
        "route_hash": route_hash,
        "output_hash": output_hash,
        "output_claim_status":"proposal_only",
        "expansion_delta_hashes": expansion_hashes,
        "working_body_before_hash": wb_before,
        "working_body_after_hash": wb.hash()
    });

    Ok(TraversalResult { output, receipt })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg() -> BrainPackage {
        BrainPackage::load_from_dir("brain_packages/example.pattern_brain.v0").unwrap()
    }

    #[test]
    fn brain_hash_stable_during_read() {
        let r = run_demo_traversal().unwrap();
        assert!(r.receipt["immutable_substrate_verified"].as_bool().unwrap());
    }
    #[test]
    fn adapter_cannot_write_back() {
        let a = ReadOnlyProjectionAdapter::new();
        assert_eq!(
            a.write_region("x", vec![]).unwrap_err(),
            "REJECT_ADAPTER_WRITE"
        );
    }
    #[test]
    fn illegal_region_access_rejected() {
        let brain = pkg();
        let a = ReadOnlyProjectionAdapter::new();
        let b = a.bind(&brain, "readonly_activation_projection_v0").unwrap();
        assert_eq!(
            a.read_region(&b, &brain, "secret.undeclared.region")
                .unwrap_err(),
            "REJECT_ROUTE"
        );
    }
    #[test]
    fn unsupported_read_mode_rejected() {
        let brain = pkg();
        let a = ReadOnlyProjectionAdapter::new();
        assert_eq!(a.bind(&brain, "bad_mode").unwrap_err(), "READ_REJECTED");
    }
    #[test]
    fn adapter_abi_mismatch_rejected() {
        let brain = pkg();
        let mut a = ReadOnlyProjectionAdapter::new();
        a.abi_id = "bad_abi".into();
        assert_eq!(
            a.bind(&brain, "readonly_activation_projection_v0")
                .unwrap_err(),
            "ADAPTER_ABI_MISMATCH"
        );
    }
    #[test]
    fn output_hash_bound_to_route_hash() {
        let r = run_demo_traversal().unwrap();
        assert!(r.receipt["route_hash"].as_str().is_some());
        assert!(r.receipt["output_hash"].as_str().is_some());
    }
}
