use std::collections::HashMap;
use std::sync::Arc;

use crate::audit::AuditLog;
use crate::bundle::path_safety::PathChecker;
use crate::bundle::repo::{BundleRepo, WriteMode};
use crate::bundle::types::*;

pub struct WriteTools {
    bundles: HashMap<String, Arc<BundleRepo>>,
    audit: Option<Arc<AuditLog>>,
}

impl WriteTools {
    pub fn new(bundles: HashMap<String, Arc<BundleRepo>>, audit: Option<Arc<AuditLog>>) -> Self {
        Self { bundles, audit }
    }

    fn get_bundle(&self, name: &str) -> Result<Arc<BundleRepo>, String> {
        self.bundles.get(name).cloned().ok_or_else(|| format!("bundle not found: {name}"))
    }

    fn audit_ok(&self, tool: &str, bundle: &str, target: &str, summary: &str) {
        if let Some(ref audit) = self.audit {
            let _ = audit.record_ok(tool, bundle, target, summary);
        }
    }

    fn audit_error(&self, tool: &str, bundle: &str, target: &str, summary: &str, error: &str) {
        if let Some(ref audit) = self.audit {
            let _ = audit.record_error(tool, bundle, target, summary, error);
        }
    }

    pub fn write_concept(
        &self,
        bundle: &str,
        concept_id: &str,
        frontmatter: Frontmatter,
        body: String,
        mode: &str,
    ) -> Result<Concept, String> {
        let repo = self.get_bundle(bundle)?;

        // Path safety check
        PathChecker::check_concept_id(concept_id).map_err(|e| e.to_string())?;

        // Reserved filename check
        let path = format!("{concept_id}.md");
        if PathChecker::is_reserved_filename(&path) {
            return Err(format!("concept ID uses reserved filename: {concept_id}"));
        }

        // Type validation
        if frontmatter.r#type.trim().is_empty() {
            return Err("frontmatter type is required and must be non-empty".to_string());
        }

        let write_mode = match mode {
            "create" => WriteMode::Create,
            "update" => WriteMode::Update,
            "upsert" => WriteMode::Upsert,
            _ => return Err(format!("invalid mode: {mode}, expected create/update/upsert")),
        };

        let id = ConceptId::new(concept_id);
        let concept = Concept {
            id: id.clone(),
            frontmatter,
            body,
        };

        let result = repo.write_concept(concept, write_mode).map_err(|e| {
            self.audit_error("okf_write_concept", bundle, concept_id, &format!("mode={mode}"), &e.to_string());
            e.to_string()
        })?;

        self.audit_ok("okf_write_concept", bundle, concept_id, &format!("mode={mode}"));
        Ok(result)
    }

    pub fn delete_concept(&self, bundle: &str, concept_id: &str) -> Result<bool, String> {
        let repo = self.get_bundle(bundle)?;

        PathChecker::check_concept_id(concept_id).map_err(|e| e.to_string())?;

        let id = ConceptId::new(concept_id);
        let result = repo.delete_concept(&id).map_err(|e| {
            self.audit_error("okf_delete_concept", bundle, concept_id, "", &e.to_string());
            e.to_string()
        })?;

        self.audit_ok("okf_delete_concept", bundle, concept_id, "");
        Ok(result)
    }

    pub fn write_index(
        &self,
        bundle: &str,
        path: &str,
        sections: Vec<IndexSection>,
        okf_version: Option<String>,
    ) -> Result<String, String> {
        let repo = self.get_bundle(bundle)?;

        let data = IndexData { sections, okf_version };
        let result = repo.write_index(path, data).map_err(|e| {
            self.audit_error("okf_write_index", bundle, path, "", &e.to_string());
            e.to_string()
        })?;

        self.audit_ok("okf_write_index", bundle, path, "");
        Ok(result)
    }

    pub fn append_log(
        &self,
        bundle: &str,
        path: &str,
        date: &str,
        entries: Vec<LogEntry>,
    ) -> Result<String, String> {
        let repo = self.get_bundle(bundle)?;

        let result = repo.append_log(path, date, &entries).map_err(|e| {
            self.audit_error("okf_append_log", bundle, path, &format!("date={date}"), &e.to_string());
            e.to_string()
        })?;

        self.audit_ok("okf_append_log", bundle, path, &format!("date={date}"));
        Ok(result)
    }

    pub fn add_citation(
        &self,
        bundle: &str,
        concept_id: &str,
        title: &str,
        target: &str,
    ) -> Result<String, String> {
        let repo = self.get_bundle(bundle)?;

        PathChecker::check_concept_id(concept_id).map_err(|e| e.to_string())?;

        let id = ConceptId::new(concept_id);
        let citation = CitationInput {
            title: title.to_string(),
            target: target.to_string(),
        };

        let result = repo.add_citation(&id, &citation).map_err(|e| {
            self.audit_error("okf_add_citation", bundle, concept_id, &format!("title={title}"), &e.to_string());
            e.to_string()
        })?;

        self.audit_ok("okf_add_citation", bundle, concept_id, &format!("title={title}"));
        Ok(result)
    }
}
