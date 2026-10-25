//! The Jev option for diagram-ir (DI-01-S1-01): semantic support review.
//!
//! No network anywhere: the "service" is a closure over the request body.

mod common;

use std::cell::Cell;

use common::{drawio_pages, fixture, mermaid_one};
use diagram_ir::semantic_review::{
    drawio_edge_claims, drawio_group_claims, mermaid_edge_claims, mermaid_group_claims,
    review_claims, review_claims_with, Aspect, Claim, Evidence, FallbackReason, JsonJudge,
    Judgement, NotJudged, ReviewPolicy, Scope, SupportJudge, SupportRequest, SupportVerdict,
    QUESTION_SET,
};
use serde_json::{json, Value};

fn scope() -> Scope {
    Scope::new("d1", "flowchart", vec!["Gateway".into(), "Queue".into()])
}

fn pair() -> (Claim, Evidence) {
    (
        Claim::edge("e1", "Gateway", "Queue", "enqueue", true),
        Evidence::new(
            "README.md#flow",
            "The gateway enqueues each request onto the queue.",
        ),
    )
}

/// A well-formed answer for `req`, with the given probabilities.
fn answer(req: &SupportRequest, p: f64, support: f64, confidence: f64) -> Value {
    let mut a = serde_json::Map::new();
    for id in req.question_ids() {
        if id == "support" {
            a.insert(id.into(), json!({"type":"score","score":support,"confidence":confidence,"legend":{},"probabilities":[]}));
        } else {
            a.insert(id.into(), json!({"type":"noul","noul":p}));
        }
    }
    json!({"model":"jev-test","usage":{"input_tokens":1,"output_tokens":1},"answers":a})
}

// --- fallback equivalence ----------------------------------------------------

/// The "pre-change" reference: the default judge.
fn baseline(claims: Vec<(Claim, Evidence)>) -> diagram_ir::semantic_review::Review {
    review_claims(&NotJudged, &scope(), claims)
}

fn assert_equivalent_to_baseline(
    got: &diagram_ir::semantic_review::Review,
    claims: Vec<(Claim, Evidence)>,
    policy: &ReviewPolicy,
    name: &str,
) {
    let base = baseline(claims);
    assert_eq!(got.items().len(), base.items().len(), "{name}");
    for (g, b) in got.items().iter().zip(base.items()) {
        assert_eq!(g.claim(), b.claim(), "{name}: claim unchanged");
        assert_eq!(g.evidence(), b.evidence(), "{name}: evidence unchanged");
        assert!(g.judgement().verdict().is_none(), "{name}: nothing judged");
    }
    assert_eq!(
        got.flagged(policy),
        base.flagged(policy),
        "{name}: no findings"
    );
    assert!(got.flagged(policy).is_empty(), "{name}");
}

#[test]
fn every_failure_mode_equals_the_default_review() {
    let claims = vec![pair(), pair()];
    let policy = ReviewPolicy {
        min_confidence: Some(0.6),
        ..ReviewPolicy::default()
    };

    // Transport-level reasons surfaced by the host.
    for reason in [
        FallbackReason::NoEndpoint,
        FallbackReason::Timeout,
        FallbackReason::Transport,
        FallbackReason::Http(500),
        FallbackReason::Http(429),
        FallbackReason::Http(529),
    ] {
        let r = reason.clone();
        let judge = JsonJudge::new("jev-test", move |_| Err(r.clone()));
        let got = review_claims_with(&judge, &scope(), claims.clone(), &policy);
        for it in got.items() {
            assert_eq!(it.judgement().fallback(), Some(&reason));
        }
        assert_equivalent_to_baseline(&got, claims.clone(), &policy, &reason.as_str());
    }

    // Bodies that parse as JSON but are wrong in each way the contract lists.
    let bad_bodies: Vec<(&str, Value)> = vec![
        ("not-an-object", json!("junk")),
        ("no-answers", json!({"model":"m"})),
        (
            "missing-id",
            json!({"answers":{"support":{"type":"score","score":2.0,"confidence":0.9}}}),
        ),
        (
            "wrong-type",
            json!({"answers":{
            "direction":{"type":"score","score":1.0,"confidence":0.9},
            "label":{"type":"noul","noul":0.9},
            "support":{"type":"score","score":2.0,"confidence":0.9}}}),
        ),
        (
            "noul-out-of-range",
            json!({"answers":{
            "direction":{"type":"noul","noul":1.5},"label":{"type":"noul","noul":0.9},
            "support":{"type":"score","score":2.0,"confidence":0.9}}}),
        ),
        (
            "score-out-of-range",
            json!({"answers":{
            "direction":{"type":"noul","noul":0.9},"label":{"type":"noul","noul":0.9},
            "support":{"type":"score","score":9.0,"confidence":0.9}}}),
        ),
        (
            "confidence-missing",
            json!({"answers":{
            "direction":{"type":"noul","noul":0.9},"label":{"type":"noul","noul":0.9},
            "support":{"type":"score","score":2.0}}}),
        ),
    ];
    for (name, body) in bad_bodies {
        let judge = JsonJudge::new("jev-test", move |_| Ok(body.clone()));
        let got = review_claims_with(&judge, &scope(), claims.clone(), &policy);
        assert!(
            got.items()
                .iter()
                .all(|i| i.judgement().fallback() == Some(&FallbackReason::Malformed)),
            "{name}"
        );
        assert_equivalent_to_baseline(&got, claims.clone(), &policy, name);
    }

    // Confident-looking but below the configured acceptance threshold.
    let judge = JsonJudge::new("jev-test", |_| Ok(json!(null)));
    drop(judge);
    struct Weak;
    impl SupportJudge for Weak {
        fn judge(&self, r: &SupportRequest) -> Result<SupportVerdict, FallbackReason> {
            r.parse_response(&answer(r, 0.02, 0.1, 0.3))
        }
    }
    let got = review_claims_with(&Weak, &scope(), claims.clone(), &policy);
    assert!(got
        .items()
        .iter()
        .all(|i| i.judgement().fallback() == Some(&FallbackReason::LowConfidence)));
    assert_equivalent_to_baseline(&got, claims.clone(), &policy, "low-confidence");
}

#[test]
fn the_default_judge_builds_no_request_and_flags_nothing() {
    struct Spy<'a>(&'a Cell<u32>);
    impl SupportJudge for Spy<'_> {
        fn is_active(&self) -> bool {
            false
        }
        fn judge(&self, _: &SupportRequest) -> Result<SupportVerdict, FallbackReason> {
            self.0.set(self.0.get() + 1);
            Err(FallbackReason::FlagOff)
        }
    }
    let calls = Cell::new(0);
    let r = review_claims(&Spy(&calls), &scope(), vec![pair()]);
    assert_eq!(calls.get(), 0, "an inactive judge is never asked");
    assert_eq!(
        r.items()[0].judgement(),
        &Judgement::NotJudged(FallbackReason::FlagOff)
    );
    let r = review_claims(&NotJudged, &scope(), vec![pair()]);
    assert_eq!(
        r.items()[0].judgement().fallback(),
        Some(&FallbackReason::FlagOff)
    );
    assert!(r
        .flagged(&ReviewPolicy {
            min_confidence: Some(0.0),
            ..ReviewPolicy::default()
        })
        .is_empty());
    assert_eq!(r.question_set(), QUESTION_SET);
}

#[test]
fn the_default_policy_records_verdicts_but_flags_nothing() {
    let judge = JsonJudge::new("m", |b: &Value| {
        // Always "reversed and unsupported".
        let n = b["questions"].as_object().unwrap().len();
        let mut a = serde_json::Map::new();
        for (k, q) in b["questions"].as_object().unwrap() {
            a.insert(
                k.clone(),
                if q["type"] == "score" {
                    json!({"type":"score","score":0.0,"confidence":1.0})
                } else {
                    json!({"type":"noul","noul":0.0})
                },
            );
        }
        assert!(n >= 2);
        Ok(json!({"answers":a}))
    });
    let r = review_claims(&judge, &scope(), vec![pair()]);
    assert!(r.items()[0].judgement().verdict().is_some());
    assert!(
        r.flagged(&ReviewPolicy::default()).is_empty(),
        "no threshold fitted ⇒ nothing flagged"
    );
}

// --- wire shape --------------------------------------------------------------

#[test]
fn edge_request_matches_the_system_one_contract() {
    let (c, e) = pair();
    let body = SupportRequest::new(&scope(), c, e).to_system_one("jev-latest");
    let obj = body.as_object().unwrap();
    assert_eq!(
        obj.keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        ["model", "state", "questions"]
            .map(String::from)
            .into_iter()
            .collect()
    );
    assert_eq!(body["model"], "jev-latest");
    let qs = body["questions"].as_object().unwrap();
    assert_eq!(
        qs.keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        ["direction", "label", "support"]
            .map(String::from)
            .into_iter()
            .collect()
    );
    for id in ["direction", "label"] {
        assert_eq!(qs[id]["type"], "noul");
        assert!(qs[id]["criteria"]["true"].is_string() && qs[id]["criteria"]["false"].is_string());
    }
    // Score: ORDERED LIST of level descriptions, no min/max range.
    assert_eq!(qs["support"]["type"], "score");
    assert_eq!(qs["support"]["criteria"].as_array().unwrap().len(), 4);
    assert!(qs["support"].get("min").is_none() && qs["support"].get("max").is_none());
    // Instructions are self-contained: they name the state fields they use.
    assert!(qs["direction"]["instructions"]
        .as_str()
        .unwrap()
        .contains("`claim.source`"));
    assert!(qs["support"]["instructions"]
        .as_str()
        .unwrap()
        .contains("`evidence.excerpt`"));
    // State carries diagram scope, the claim and the exact evidence.
    assert_eq!(body["state"]["claim"]["source"], "Gateway");
    assert_eq!(body["state"]["claim"]["directed"], true);
    assert_eq!(body["state"]["evidence"]["source"], "README.md#flow");
    assert_eq!(
        body["state"]["diagram"]["node_labels"],
        json!(["Gateway", "Queue"])
    );
}

#[test]
fn only_applicable_questions_are_asked() {
    // Undirected, unlabelled edge: only overall support.
    let r = SupportRequest::new(
        &scope(),
        Claim::edge("e", "A", "B", "  ", false),
        Evidence::new("s", "x"),
    );
    assert_eq!(r.question_ids(), vec!["support"]);
    // Directed but unlabelled.
    let r = SupportRequest::new(
        &scope(),
        Claim::edge("e", "A", "B", "", true),
        Evidence::new("s", "x"),
    );
    assert_eq!(r.question_ids(), vec!["direction", "support"]);
    // Grouping.
    let r = SupportRequest::new(
        &scope(),
        Claim::group("n", "Web App", "Edge"),
        Evidence::new("s", "x"),
    );
    assert_eq!(r.question_ids(), vec!["grouping", "support"]);
    let body = r.to_system_one("m");
    assert_eq!(body["questions"]["grouping"]["type"], "noul");
    assert_eq!(body["state"]["claim"]["group"], "Edge");
}

#[test]
fn requests_are_bounded() {
    let long = "x".repeat(10_000);
    let labels: Vec<String> = (0..500).map(|i| format!("{i}{long}")).collect();
    let s = Scope::new("d", "flowchart", labels);
    let r = SupportRequest::new(
        &s,
        Claim::edge("e", "A", "B", "l", true),
        Evidence::new("s", long),
    );
    let b = r.to_system_one("m");
    assert_eq!(
        b["state"]["evidence"]["excerpt"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        2000
    );
    let ls = b["state"]["diagram"]["node_labels"].as_array().unwrap();
    assert_eq!(ls.len(), 40);
    assert!(ls
        .iter()
        .all(|l| l.as_str().unwrap().chars().count() <= 120));
}

// --- accept path over real IR ------------------------------------------------

/// A stand-in service: judges "supported" iff the excerpt mentions the
/// source label before the target label (a crude direction oracle).
fn oracle(body: &Value) -> Result<Value, FallbackReason> {
    let ex = body["state"]["evidence"]["excerpt"]
        .as_str()
        .unwrap()
        .to_lowercase();
    let s = body["state"]["claim"]["source"]
        .as_str()
        .unwrap_or("")
        .to_lowercase();
    let t = body["state"]["claim"]["target"]
        .as_str()
        .unwrap_or("")
        .to_lowercase();
    let forward = match (ex.find(&s), ex.find(&t)) {
        (Some(a), Some(b)) => a < b,
        _ => false,
    };
    let mut a = serde_json::Map::new();
    for (k, q) in body["questions"].as_object().unwrap() {
        a.insert(k.clone(), if q["type"] == "score" {
            json!({"type":"score","score": if forward {2.9} else {0.4},"confidence":0.9,"legend":{},"probabilities":[]})
        } else {
            json!({"type":"noul","noul": if forward {0.97} else {0.03}})
        });
    }
    Ok(json!({"model":"jev-test","usage":{"input_tokens":1,"output_tokens":1},"answers":a}))
}

#[test]
fn a_seeded_reversed_edge_is_flagged_and_a_true_one_is_not() {
    let d =
        mermaid_one("flowchart LR\n  G[Gateway] -->|enqueue| Q[Queue]\n  Q -->|drain| W[Worker]\n");
    let first = d.edges[0].id.clone();
    let ev = |id: &str| {
        if id == first {
            // Matches the diagram: Gateway -> Queue.
            Some(Evidence::new("flow.md", "Gateway enqueues onto Queue"))
        } else {
            // Seeded reversal: the evidence has the Worker draining the Queue.
            Some(Evidence::new(
                "flow.md",
                "Worker pulls work; the Worker drains the Queue",
            ))
        }
    };
    let claims = mermaid_edge_claims(&d, &ev);
    assert_eq!(claims.len(), 2, "both edges resolve their endpoint labels");
    let judge = JsonJudge::new("jev-test", oracle);
    let policy = ReviewPolicy {
        min_confidence: Some(0.5),
        ..ReviewPolicy::default()
    };
    let review = review_claims_with(&judge, &Scope::from_mermaid(&d), claims, &policy);
    let findings = review.flagged(&policy);
    let ids: Vec<_> = findings.iter().map(|f| f.claim_id.clone()).collect();
    let reversed = &d.edges[1].id;
    assert!(
        ids.iter().all(|i| i == reversed),
        "only the reversed edge: {ids:?}"
    );
    assert!(findings.iter().any(|f| f.aspect == Aspect::Direction));
    assert!(findings.iter().any(|f| f.aspect == Aspect::Support));
    assert!(findings.iter().all(|f| f.evidence_source == "flow.md"));
    // Extraction itself is untouched by the review.
    assert_eq!(d.edges.len(), 2);
}

#[test]
fn drawio_edges_and_groups_become_claims() {
    let pages = drawio_pages(&fixture("bare-model.drawio")).unwrap();
    let page = &pages[0];
    let claims = drawio_edge_claims(page, &|id| {
        Some(Evidence::new("doc", format!("about {id}")))
    });
    assert_eq!(claims.len(), page.edges.len());
    match &claims[0].0 {
        Claim::Edge {
            source,
            target,
            label,
            directed,
            ..
        } => {
            assert_eq!(
                (source.as_str(), target.as_str(), label.as_str(), *directed),
                ("Ingress", "Service & Queue", "enqueue", true)
            );
        }
        other => panic!("{other:?}"),
    }
    // No evidence ⇒ no question.
    assert!(drawio_edge_claims(page, &|_| None).is_empty());

    let arch = drawio_pages(&fixture("sample-architecture.drawio")).unwrap();
    let groups = drawio_group_claims(&arch[0], &|id| Some(Evidence::new("doc", id.to_owned())));
    assert!(groups
        .iter()
        .any(|(c, _)| matches!(c, Claim::Group { group, .. } if group == "Edge")));
    let s = Scope::from_drawio(&arch[0]);
    let body = SupportRequest::new(&s, groups[0].0.clone(), groups[0].1.clone()).to_system_one("m");
    assert_eq!(body["questions"]["grouping"]["type"], "noul");
}

#[test]
fn mermaid_subgraphs_become_group_claims() {
    let d = mermaid_one("flowchart TD\n  subgraph Core\n    A[Api]\n    B[Db]\n  end\n  A --> B\n");
    let groups = mermaid_group_claims(&d, &|id| Some(Evidence::new("doc", id.to_owned())));
    assert_eq!(groups.len(), 2);
    assert!(groups
        .iter()
        .all(|(c, _)| matches!(c, Claim::Group { group, .. } if group == "Core")));
}

#[test]
fn a_confident_verdict_is_parsed_into_the_documented_fields() {
    let (c, e) = pair();
    let req = SupportRequest::new(&scope(), c, e);
    let v = req.parse_response(&answer(&req, 0.9, 2.7, 0.8)).unwrap();
    assert_eq!(v.direction, Some(0.9));
    assert_eq!(v.label, Some(0.9));
    assert_eq!(v.grouping, None);
    assert!((v.support - 2.7).abs() < 1e-9 && (v.confidence - 0.8).abs() < 1e-9);
}
