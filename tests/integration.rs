use std::sync::Arc;

use okf_mcp_server::bundle::fs_store::LocalFsStore;
use okf_mcp_server::bundle::repo::{BundleRepo, WriteMode};
use okf_mcp_server::bundle::store::BundleStore;
use okf_mcp_server::bundle::types::*;

fn setup_test_bundle(name: &str) -> (tempfile::TempDir, BundleRepo) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let store: Arc<dyn BundleStore> = Arc::new(LocalFsStore::new(root.clone()));
    let repo = BundleRepo::new(name.to_string(), store, root);
    (dir, repo)
}

#[test]
fn test_write_and_read_concept() {
    let (_dir, repo) = setup_test_bundle("test");

    let concept = Concept {
        id: ConceptId::new("tables/orders"),
        frontmatter: Frontmatter {
            r#type: "BigQuery Table".to_string(),
            title: Some("Orders".to_string()),
            description: Some("Orders table".to_string()),
            resource: None,
            tags: Some(vec!["billing".to_string(), "ecommerce".to_string()]),
            timestamp: Some("2025-01-01T00:00:00Z".to_string()),
            extra: serde_yaml::Mapping::new(),
        },
        body: "# Orders\n\nThis is the orders table.\n\n# Schema\n\n| column | type |\n|--------|------|\n| id | INT64 |\n".to_string(),
    };

    repo.write_concept(concept.clone(), WriteMode::Create).unwrap();

    let read = repo.read_concept(&ConceptId::new("tables/orders")).unwrap();
    assert_eq!(read.id.to_string(), "tables/orders");
    assert_eq!(read.frontmatter.r#type, "BigQuery Table");
    assert_eq!(read.frontmatter.title.unwrap(), "Orders");
    assert!(read.body.contains("orders table"));
}

#[test]
fn test_list_concepts() {
    let (_dir, repo) = setup_test_bundle("test");

    let ids = vec!["tables/orders", "tables/customers", "views/revenue"];
    for id in &ids {
        let concept = Concept {
            id: ConceptId::new(*id),
            frontmatter: Frontmatter {
                r#type: "Table".to_string(),
                title: Some(id.to_string()),
                description: None,
                resource: None,
                tags: None,
                timestamp: None,
                extra: serde_yaml::Mapping::new(),
            },
            body: format!("# {id}"),
        };
        repo.write_concept(concept, WriteMode::Create).unwrap();
    }

    let all = repo.list_concepts(None, None, None).unwrap();
    assert_eq!(all.len(), 3);

    let prefix = repo.list_concepts(Some("tables"), None, None).unwrap();
    assert_eq!(prefix.len(), 2);
}

#[test]
fn test_delete_concept() {
    let (_dir, repo) = setup_test_bundle("test");

    let concept = Concept {
        id: ConceptId::new("tables/orders"),
        frontmatter: Frontmatter {
            r#type: "Table".to_string(),
            title: None,
            description: None,
            resource: None,
            tags: None,
            timestamp: None,
            extra: serde_yaml::Mapping::new(),
        },
        body: "content".to_string(),
    };
    repo.write_concept(concept, WriteMode::Create).unwrap();
    assert_eq!(repo.list_concepts(None, None, None).unwrap().len(), 1);

    repo.delete_concept(&ConceptId::new("tables/orders")).unwrap();
    assert_eq!(repo.list_concepts(None, None, None).unwrap().len(), 0);
}

#[test]
fn test_validate_bundle() {
    let (_dir, repo) = setup_test_bundle("test");

    // Valid concept
    let concept = Concept {
        id: ConceptId::new("valid"),
        frontmatter: Frontmatter {
            r#type: "ValidType".to_string(),
            title: Some("Valid".to_string()),
            description: None,
            resource: None,
            tags: None,
            timestamp: None,
            extra: serde_yaml::Mapping::new(),
        },
        body: "Body".to_string(),
    };
    repo.write_concept(concept, WriteMode::Create).unwrap();

    let result = repo.validate().unwrap();
    assert_eq!(result.errors.len(), 0, "expected no errors, got: {:?}", result.errors);
}

#[test]
fn test_validate_missing_type() {
    let (_dir, repo) = setup_test_bundle("test");

    let store = repo.store();
    store.write_raw("bad.md", "---\ntype: \n---\n\nBody").unwrap();

    let result = repo.validate().unwrap();
    assert!(result.errors.iter().any(|e| e.contains("empty type")), "expected empty type error, got: {:?}", result.errors);
}

#[test]
fn test_read_index_synthesized_root() {
    let (_dir, repo) = setup_test_bundle("test");

    let concept = Concept {
        id: ConceptId::new("orders"),
        frontmatter: Frontmatter {
            r#type: "Table".to_string(),
            title: Some("Orders".to_string()),
            description: Some("Order data".to_string()),
            resource: None,
            tags: None,
            timestamp: None,
            extra: serde_yaml::Mapping::new(),
        },
        body: "Body".to_string(),
    };
    repo.write_concept(concept, WriteMode::Create).unwrap();

    let result = repo.read_index("").unwrap();
    assert!(result.rendered.contains("Orders"), "root index should contain root-level concept: {}", result.rendered);
    assert!(result.sections.is_some());
}

#[test]
fn test_read_index_synthesized_subdirectory() {
    let (_dir, repo) = setup_test_bundle("test");

    let concept = Concept {
        id: ConceptId::new("tables/orders"),
        frontmatter: Frontmatter {
            r#type: "Table".to_string(),
            title: Some("Orders".to_string()),
            description: Some("Order data".to_string()),
            resource: None,
            tags: None,
            timestamp: None,
            extra: serde_yaml::Mapping::new(),
        },
        body: "Body".to_string(),
    };
    repo.write_concept(concept, WriteMode::Create).unwrap();

    // Root index should show subdirectory "tables"
    let root_result = repo.read_index("").unwrap();
    assert!(root_result.rendered.contains("tables"), "root index should contain subdirectory: {}", root_result.rendered);

    // Subdirectory index should show the concept
    let sub_result = repo.read_index("tables").unwrap();
    assert!(sub_result.rendered.contains("Orders"), "tables index should contain concept: {}", sub_result.rendered);
}

#[test]
fn test_path_safety_rejects_dot_dot() {
    let (_dir, repo) = setup_test_bundle("test");

    let id = ConceptId::new("../../etc/passwd");
    let concept = Concept {
        id: id.clone(),
        frontmatter: Frontmatter {
            r#type: "Type".to_string(),
            title: None,
            description: None,
            resource: None,
            tags: None,
            timestamp: None,
            extra: serde_yaml::Mapping::new(),
        },
        body: "Malicious".to_string(),
    };

    let result = repo.write_concept(concept, WriteMode::Create);
    assert!(result.is_err(), "expected path traversal to be rejected");
}

#[test]
fn test_frontmatter_round_trip_extra_keys() {
    let (_dir, repo) = setup_test_bundle("test");

    // Write with extra keys
    let mut extra = serde_yaml::Mapping::new();
    extra.insert(
        serde_yaml::Value::String("custom_key".to_string()),
        serde_yaml::Value::String("custom_value".to_string()),
    );
    extra.insert(
        serde_yaml::Value::String("nested".to_string()),
        serde_yaml::Value::Mapping({
            let mut m = serde_yaml::Mapping::new();
            m.insert(
                serde_yaml::Value::String("inner".to_string()),
                serde_yaml::Value::Number(42.into()),
            );
            m
        }),
    );

    let concept = Concept {
        id: ConceptId::new("extra_test"),
        frontmatter: Frontmatter {
            r#type: "Type".to_string(),
            title: None,
            description: None,
            resource: None,
            tags: None,
            timestamp: None,
            extra,
        },
        body: "Body".to_string(),
    };
    repo.write_concept(concept, WriteMode::Create).unwrap();

    let read = repo.read_concept(&ConceptId::new("extra_test")).unwrap();
    assert_eq!(
        read.frontmatter.extra.get("custom_key").and_then(|v| v.as_str()),
        Some("custom_value")
    );
    assert!(read.frontmatter.extra.contains_key("nested"));
}

#[test]
fn test_log_append() {
    let (_dir, repo) = setup_test_bundle("test");

    let entries = vec![
        LogEntry {
            label: Some("Creation".to_string()),
            text: "Created the bundle".to_string(),
        },
        LogEntry {
            label: None,
            text: "Added initial concepts".to_string(),
        },
    ];

    let result = repo.append_log("", "2025-01-15", &entries).unwrap();
    assert!(result.contains("## 2025-01-15"));
    assert!(result.contains("**Creation**"));
    assert!(result.contains("Created the bundle"));
    assert!(result.contains("Added initial concepts"));

    // Append more under same date
    let more = vec![LogEntry {
        label: Some("Update".to_string()),
        text: "Fixed a typo".to_string(),
    }];
    let updated = repo.append_log("", "2025-01-15", &more).unwrap();
    assert!(updated.contains("**Update**"));
    assert!(updated.contains("Fixed a typo"));
}

#[test]
fn test_citation_add() {
    let (_dir, repo) = setup_test_bundle("test");

    let concept = Concept {
        id: ConceptId::new("citable"),
        frontmatter: Frontmatter {
            r#type: "Type".to_string(),
            title: Some("Citable".to_string()),
            description: None,
            resource: None,
            tags: None,
            timestamp: None,
            extra: serde_yaml::Mapping::new(),
        },
        body: "# My Concept\n\nSome content.\n".to_string(),
    };
    repo.write_concept(concept, WriteMode::Create).unwrap();

    let citation = CitationInput {
        title: "Reference Doc".to_string(),
        target: "https://example.com/doc".to_string(),
    };
    let updated = repo.add_citation(&ConceptId::new("citable"), &citation).unwrap();
    assert!(updated.contains("# Citations"));
    assert!(updated.contains("[1]"));
    assert!(updated.contains("Reference Doc"));

    // Add another citation
    let citation2 = CitationInput {
        title: "Another Ref".to_string(),
        target: "https://example.com/ref2".to_string(),
    };
    let updated2 = repo.add_citation(&ConceptId::new("citable"), &citation2).unwrap();
    assert!(updated2.contains("[2]"));
    assert!(updated2.contains("Another Ref"));
}

#[test]
fn test_search() {
    let (_dir, repo) = setup_test_bundle("test");

    let concepts = vec![
        ("orders", "BigQuery Table", "Order processing", "billing"),
        ("customers", "BigQuery Table", "Customer data", "crm"),
        ("revenue", "View", "Revenue analysis", "finance"),
    ];

    for (id, typ, desc, tag) in &concepts {
        let concept = Concept {
            id: ConceptId::new(*id),
            frontmatter: Frontmatter {
                r#type: typ.to_string(),
                title: Some(id.to_string()),
                description: Some(desc.to_string()),
                resource: None,
                tags: Some(vec![tag.to_string()]),
                timestamp: None,
                extra: serde_yaml::Mapping::new(),
            },
            body: format!("# {}\n\nThis concept describes {}.", id, desc),
        };
        repo.write_concept(concept, WriteMode::Create).unwrap();
    }

    let results = repo.search("orders", None, None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].concept_id, "orders");

    let customer_results = repo.search("Customer", None, None).unwrap();
    assert_eq!(customer_results.len(), 1);
    assert_eq!(customer_results[0].concept_id, "customers");
}

#[test]
fn test_get_backlinks() {
    let (_dir, repo) = setup_test_bundle("test");

    // Create two concepts, one linking to the other
    let target = Concept {
        id: ConceptId::new("target"),
        frontmatter: Frontmatter {
            r#type: "Type".to_string(),
            title: None,
            description: None,
            resource: None,
            tags: None,
            timestamp: None,
            extra: serde_yaml::Mapping::new(),
        },
        body: "Target concept".to_string(),
    };
    repo.write_concept(target, WriteMode::Create).unwrap();

    let source = Concept {
        id: ConceptId::new("source"),
        frontmatter: Frontmatter {
            r#type: "Type".to_string(),
            title: None,
            description: None,
            resource: None,
            tags: None,
            timestamp: None,
            extra: serde_yaml::Mapping::new(),
        },
        body: "This links to [target](/target.md).".to_string(),
    };
    repo.write_concept(source, WriteMode::Create).unwrap();

    let backlinks = repo.get_backlinks(&ConceptId::new("target")).unwrap();
    assert_eq!(backlinks.len(), 1);
    assert_eq!(backlinks[0].to_string(), "source");
}
