//! Optional **semantic support review** over extracted diagram IR (the *Jev
//! option*, item DI-01-S1-01).
//!
//! Extraction in this crate is deterministic and syntactic: it can tell you
//! that an edge runs from `A` to `B` and carries the label `calls`, but not
//! whether the source material actually says so. This module adds a meaning
//! check that a *consumer* may run beside extraction: given one claim taken
//! from the IR (an edge's direction and label, or a node's grouping) and the
//! exact evidence the caller supplies for it, ask a typed-decision service
//! whether the evidence supports the claim.
//!
//! # What this module is, and is not
//!
//! * It performs **no network I/O** and adds no dependencies. The decision
//!   service is behind the [`SupportJudge`] trait; the host supplies the
//!   transport (see [`JsonJudge`]). The default judge, [`NotJudged`], builds
//!   no request and returns [`FallbackReason::FlagOff`] for every claim.
//! * It never changes extraction. Parsers, limits and IR types are untouched;
//!   a review is a separate value, and an "all not judged" review is exactly
//!   the pre-change state.
//! * It never certifies a diagram. A supported claim means "this evidence
//!   states it", not "the architecture is correct". Candidate generation (which
//!   claims, which evidence) stays in caller code; only the semantic judgement
//!   is delegated.
//! * It reports annotations. By default ([`ReviewPolicy::default`]) nothing is
//!   *flagged*: a verdict is recorded, but a threshold fitted on labelled data
//!   must be configured before [`Review::flagged`] returns anything.
//!
//! # Wire contract
//!
//! [`SupportRequest::to_system_one`] builds a System One
//! (`POST /v1/systemone`) body: independent `noul` questions for each
//! obligation (direction, label, grouping) and a `score` question with an
//! ordered four-level rubric for overall support. Question IDs are not shown to
//! the model, so every instruction carries its full meaning and names the state
//! fields it refers to. The question-set version is [`QUESTION_SET`].
//!
//! # Example
//!
//! ```
//! use diagram_ir::semantic_review::{
//!     review_claims, Claim, Evidence, NotJudged, ReviewPolicy, Scope,
//! };
//!
//! let scope = Scope::new("d1", "flowchart", vec!["Gateway".into(), "Queue".into()]);
//! let claim = Claim::edge("e1", "Gateway", "Queue", "enqueue", true);
//! let evidence = Evidence::new("README.md#flow", "The gateway enqueues each request.");
//!
//! // Default judge: no request is built and every claim is "not judged".
//! let review = review_claims(&NotJudged, &scope, vec![(claim, evidence)]);
//! assert!(review.items()[0].judgement().verdict().is_none());
//! assert!(review.flagged(&ReviewPolicy::default()).is_empty());
//! ```

use std::fmt;

use serde_json::{json, Map, Value};

use crate::drawio::model::Page;
use crate::mermaid::model::Diagram;

/// Version string of the question set built by this module. Log it with every
/// call so results can be tied to the exact instructions that produced them.
pub const QUESTION_SET: &str = "diagram-ir.di-01-s1-01.v1";

/// Maximum characters of evidence excerpt placed in a request.
pub const MAX_EXCERPT_CHARS: usize = 2000;
/// Maximum node labels listed as diagram scope in a request.
pub const MAX_SCOPE_LABELS: usize = 40;
/// Maximum characters of any one label placed in a request.
pub const MAX_LABEL_CHARS: usize = 120;

/// Why a claim was not judged. Every variant means "behave as if no review
/// had been requested"; [`FallbackReason::as_str`] is the only thing a host
/// should log (never the request content).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FallbackReason {
    /// The option is off (the [`NotJudged`] default).
    FlagOff,
    /// The host has no endpoint to call.
    NoEndpoint,
    /// The host's deadline elapsed.
    Timeout,
    /// The host could not reach the service.
    Transport,
    /// The service answered with a non-2xx status.
    Http(u16),
    /// The body did not parse, an expected answer was missing or out of range.
    Malformed,
    /// The verdict's confidence was below [`ReviewPolicy::min_confidence`].
    LowConfidence,
}

impl FallbackReason {
    /// Stable log token: `flag-off`, `no-endpoint`, `timeout`, `transport`,
    /// `http-<code>`, `malformed` or `low-confidence`.
    ///
    /// ```
    /// use diagram_ir::semantic_review::FallbackReason;
    /// assert_eq!(FallbackReason::Http(429).as_str(), "http-429");
    /// assert_eq!(FallbackReason::LowConfidence.as_str(), "low-confidence");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> String {
        match self {
            Self::FlagOff => "flag-off".to_owned(),
            Self::NoEndpoint => "no-endpoint".to_owned(),
            Self::Timeout => "timeout".to_owned(),
            Self::Transport => "transport".to_owned(),
            Self::Http(code) => format!("http-{code}"),
            Self::Malformed => "malformed".to_owned(),
            Self::LowConfidence => "low-confidence".to_owned(),
        }
    }
}

impl fmt::Display for FallbackReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// The exact source evidence offered for one claim.
///
/// The caller chooses it (retrieval stays deterministic and in caller code);
/// the excerpt is clipped to [`MAX_EXCERPT_CHARS`] when a request is built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    source: String,
    excerpt: String,
}

impl Evidence {
    /// Evidence from `source` (a path, anchor or document id) with the verbatim
    /// `excerpt` that is claimed to state the relationship.
    #[must_use]
    pub fn new(source: impl Into<String>, excerpt: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            excerpt: excerpt.into(),
        }
    }

    /// The source identifier.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The excerpt as supplied (unclipped).
    #[must_use]
    pub fn excerpt(&self) -> &str {
        &self.excerpt
    }
}

/// The diagram a claim belongs to: enough context for the model to resolve
/// names, bounded so a large diagram cannot inflate a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    id: String,
    kind: String,
    labels: Vec<String>,
}

impl Scope {
    /// A scope with a stable diagram `id`, its `kind` (for example
    /// `flowchart`) and the node labels visible in it.
    #[must_use]
    pub fn new(id: impl Into<String>, kind: impl Into<String>, labels: Vec<String>) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            labels,
        }
    }

    /// Scope for a Mermaid diagram.
    #[must_use]
    pub fn from_mermaid(diagram: &Diagram) -> Self {
        Self::new(
            format!("mermaid-{}", diagram.index),
            diagram.kind.clone(),
            diagram.nodes.iter().map(|n| n.label.clone()).collect(),
        )
    }

    /// Scope for a draw.io page.
    #[must_use]
    pub fn from_drawio(page: &Page) -> Self {
        Self::new(
            page.id.clone(),
            "drawio",
            page.nodes.iter().map(|n| n.label.clone()).collect(),
        )
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind,
            "node_labels": self.labels.iter().take(MAX_SCOPE_LABELS)
                .map(|l| clip(l, MAX_LABEL_CHARS)).collect::<Vec<_>>(),
        })
    }
}

/// One claim read off the IR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Claim {
    /// An edge: its direction (only when `directed`) and its label (only when
    /// non-empty) are the obligations checked.
    Edge {
        /// The IR edge id.
        id: String,
        /// Label of the source node.
        source: String,
        /// Label of the target node.
        target: String,
        /// The edge label, possibly empty.
        label: String,
        /// Whether the IR says the edge has a single direction.
        directed: bool,
    },
    /// A node placed inside a container.
    Group {
        /// The member node id.
        id: String,
        /// Label of the member node.
        member: String,
        /// Label of the containing node.
        group: String,
    },
}

impl Claim {
    /// An edge claim.
    #[must_use]
    pub fn edge(
        id: impl Into<String>,
        source: impl Into<String>,
        target: impl Into<String>,
        label: impl Into<String>,
        directed: bool,
    ) -> Self {
        Self::Edge {
            id: id.into(),
            source: source.into(),
            target: target.into(),
            label: label.into(),
            directed,
        }
    }

    /// A grouping claim.
    #[must_use]
    pub fn group(
        id: impl Into<String>,
        member: impl Into<String>,
        group: impl Into<String>,
    ) -> Self {
        Self::Group {
            id: id.into(),
            member: member.into(),
            group: group.into(),
        }
    }

    /// The IR id this claim is about.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Edge { id, .. } | Self::Group { id, .. } => id,
        }
    }
}

/// Edge claims for a draw.io page. `evidence` is asked for each edge id; edges
/// with no evidence, a dangling end, or no labelled endpoint are skipped (no
/// evidence, no question).
pub fn drawio_edge_claims(
    page: &Page,
    evidence: &dyn Fn(&str) -> Option<Evidence>,
) -> Vec<(Claim, Evidence)> {
    let label_of = |id: &Option<String>| {
        let id = id.as_deref()?;
        page.nodes
            .iter()
            .find(|n| n.id == id)
            .map(|n| n.label.clone())
    };
    page.edges
        .iter()
        .filter_map(|e| {
            let (s, t) = (label_of(&e.source)?, label_of(&e.target)?);
            let ev = evidence(&e.id)?;
            Some((
                Claim::edge(&e.id, s, t, &e.label, !(e.undirected || e.bidirectional)),
                ev,
            ))
        })
        .collect()
}

/// Edge claims for a Mermaid diagram; see [`drawio_edge_claims`].
pub fn mermaid_edge_claims(
    diagram: &Diagram,
    evidence: &dyn Fn(&str) -> Option<Evidence>,
) -> Vec<(Claim, Evidence)> {
    let label_of = |id: &str| {
        diagram
            .nodes
            .iter()
            .find(|n| n.id == id)
            .map(|n| n.label.clone())
    };
    diagram
        .edges
        .iter()
        .filter_map(|e| {
            let (s, t) = (label_of(&e.source)?, label_of(&e.target)?);
            let ev = evidence(&e.id)?;
            Some((
                Claim::edge(&e.id, s, t, &e.label, !(e.undirected || e.bidirectional)),
                ev,
            ))
        })
        .collect()
}

/// Grouping claims for a draw.io page: every node whose parent is a container
/// node of the page. `evidence` is asked for the member node id.
pub fn drawio_group_claims(
    page: &Page,
    evidence: &dyn Fn(&str) -> Option<Evidence>,
) -> Vec<(Claim, Evidence)> {
    page.nodes
        .iter()
        .filter_map(|n| {
            let parent = n.parent.as_deref()?;
            let g = page.nodes.iter().find(|p| p.id == parent && p.container)?;
            let ev = evidence(&n.id)?;
            Some((Claim::group(&n.id, &n.label, &g.label), ev))
        })
        .collect()
}

/// Grouping claims for a Mermaid diagram (subgraphs / composite states).
pub fn mermaid_group_claims(
    diagram: &Diagram,
    evidence: &dyn Fn(&str) -> Option<Evidence>,
) -> Vec<(Claim, Evidence)> {
    diagram
        .nodes
        .iter()
        .filter_map(|n| {
            let parent = n.parent.as_deref()?;
            let g = diagram
                .nodes
                .iter()
                .find(|p| p.id == parent && p.container)?;
            let ev = evidence(&n.id)?;
            Some((Claim::group(&n.id, &n.label, &g.label), ev))
        })
        .collect()
}

/// A built request for one claim, ready to be turned into a wire body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupportRequest {
    scope: Scope,
    claim: Claim,
    evidence: Evidence,
}

/// The answer IDs a request asks, in the order they appear on the wire.
const Q_DIRECTION: &str = "direction";
const Q_LABEL: &str = "label";
const Q_GROUPING: &str = "grouping";
const Q_SUPPORT: &str = "support";

impl SupportRequest {
    /// Build the request for `claim` with its `evidence` in `scope`.
    #[must_use]
    pub fn new(scope: &Scope, claim: Claim, evidence: Evidence) -> Self {
        Self {
            scope: scope.clone(),
            claim,
            evidence,
        }
    }

    /// The claim this request is about.
    #[must_use]
    pub fn claim(&self) -> &Claim {
        &self.claim
    }

    /// The question IDs this request asks.
    #[must_use]
    pub fn question_ids(&self) -> Vec<&'static str> {
        match &self.claim {
            Claim::Edge {
                label, directed, ..
            } => {
                let mut v = Vec::new();
                if *directed {
                    v.push(Q_DIRECTION);
                }
                if !label.trim().is_empty() {
                    v.push(Q_LABEL);
                }
                v.push(Q_SUPPORT);
                v
            }
            Claim::Group { .. } => vec![Q_GROUPING, Q_SUPPORT],
        }
    }

    /// The System One request body for `model`.
    ///
    /// Obligations are independent `noul` questions; overall support is one
    /// `score` question whose `criteria` is an ordered list. Instructions name
    /// the `claim`, `evidence` and `diagram` state fields explicitly because
    /// question IDs are not shown to the model.
    #[must_use]
    pub fn to_system_one(&self, model: &str) -> Value {
        let mut questions = Map::new();
        let claim_json = match &self.claim {
            Claim::Edge {
                id,
                source,
                target,
                label,
                directed,
            } => {
                if *directed {
                    questions.insert(Q_DIRECTION.to_owned(), json!({
                        "type": "noul",
                        "instructions": "In `evidence.excerpt`, the relationship between `claim.source` and `claim.target` runs from `claim.source` to `claim.target` (the source calls, sends to, depends on, precedes or acts on the target), and not the other way round.",
                        "criteria": {
                            "true": "The excerpt states or clearly implies the relationship runs from claim.source to claim.target.",
                            "false": "The excerpt states the reverse direction, or says nothing about direction."
                        }
                    }));
                }
                if !label.trim().is_empty() {
                    questions.insert(Q_LABEL.to_owned(), json!({
                        "type": "noul",
                        "instructions": "The text in `claim.label` correctly names the relationship between `claim.source` and `claim.target` according to `evidence.excerpt`.",
                        "criteria": {
                            "true": "The excerpt describes the same relationship that claim.label names.",
                            "false": "The excerpt describes a different relationship, or does not mention one."
                        }
                    }));
                }
                json!({ "kind": "edge", "edge_id": id, "source": clip(source, MAX_LABEL_CHARS),
                        "target": clip(target, MAX_LABEL_CHARS), "label": clip(label, MAX_LABEL_CHARS),
                        "directed": directed })
            }
            Claim::Group { id, member, group } => {
                questions.insert(Q_GROUPING.to_owned(), json!({
                    "type": "noul",
                    "instructions": "According to `evidence.excerpt`, `claim.member` belongs to, is part of or is owned by `claim.group`.",
                    "criteria": {
                        "true": "The excerpt places claim.member inside claim.group.",
                        "false": "The excerpt places it elsewhere, or does not relate it to claim.group."
                    }
                }));
                json!({ "kind": "group", "member_id": id, "member": clip(member, MAX_LABEL_CHARS),
                        "group": clip(group, MAX_LABEL_CHARS) })
            }
        };
        questions.insert(Q_SUPPORT.to_owned(), json!({
            "type": "score",
            "instructions": "How directly does `evidence.excerpt` state the whole relationship asserted by `claim`, including names, direction and label where present? Judge only what the excerpt says, not what is plausible.",
            "criteria": [
                "The excerpt contradicts the claim",
                "The excerpt does not mention the claim",
                "The excerpt implies the claim but does not state it",
                "The excerpt states the claim explicitly"
            ]
        }));
        json!({
            "model": model,
            "state": {
                "diagram": self.scope.to_json(),
                "claim": claim_json,
                "evidence": {
                    "source": self.evidence.source,
                    "excerpt": clip(&self.evidence.excerpt, MAX_EXCERPT_CHARS),
                },
            },
            "questions": Value::Object(questions),
        })
    }

    /// Parse a System One response for this request.
    ///
    /// # Errors
    /// [`FallbackReason::Malformed`] unless every asked answer is present with
    /// the right type and a value in range: `noul` in `[0, 1]`, `score` in
    /// `[0, 3]`, `confidence` in `[0, 1]`.
    pub fn parse_response(&self, body: &Value) -> Result<SupportVerdict, FallbackReason> {
        let answers = body
            .get("answers")
            .and_then(Value::as_object)
            .ok_or(FallbackReason::Malformed)?;
        let noul = |id: &str| -> Result<f64, FallbackReason> {
            let a = answers.get(id).ok_or(FallbackReason::Malformed)?;
            if a.get("type").and_then(Value::as_str) != Some("noul") {
                return Err(FallbackReason::Malformed);
            }
            a.get("noul")
                .and_then(Value::as_f64)
                .filter(|p| (0.0..=1.0).contains(p))
                .ok_or(FallbackReason::Malformed)
        };
        let ids = self.question_ids();
        let opt = |id: &'static str| -> Result<Option<f64>, FallbackReason> {
            if ids.contains(&id) {
                noul(id).map(Some)
            } else {
                Ok(None)
            }
        };
        let (direction, label, grouping) = (opt(Q_DIRECTION)?, opt(Q_LABEL)?, opt(Q_GROUPING)?);
        let s = answers.get(Q_SUPPORT).ok_or(FallbackReason::Malformed)?;
        if s.get("type").and_then(Value::as_str) != Some("score") {
            return Err(FallbackReason::Malformed);
        }
        let support = s
            .get("score")
            .and_then(Value::as_f64)
            .filter(|v| (0.0..=3.0).contains(v))
            .ok_or(FallbackReason::Malformed)?;
        let confidence = s
            .get("confidence")
            .and_then(Value::as_f64)
            .filter(|c| (0.0..=1.0).contains(c))
            .ok_or(FallbackReason::Malformed)?;
        Ok(SupportVerdict {
            direction,
            label,
            grouping,
            support,
            confidence,
        })
    }
}

/// What the service said about one claim.
#[derive(Clone, Debug, PartialEq)]
pub struct SupportVerdict {
    /// Probability that the evidence supports the stated direction.
    pub direction: Option<f64>,
    /// Probability that the evidence supports the label.
    pub label: Option<f64>,
    /// Probability that the evidence supports the grouping.
    pub grouping: Option<f64>,
    /// Expected overall support level in `[0, 3]` (3 = stated explicitly).
    pub support: f64,
    /// Confidence of the `support` score, in `[0, 1]`.
    pub confidence: f64,
}

/// A source of semantic judgements. Implemented by the host.
///
/// Implementations must not panic and should return quickly; any failure is
/// reported as a [`FallbackReason`], never as a partial verdict.
pub trait SupportJudge {
    /// Whether this judge will ever answer. When `false`, [`review_claims`]
    /// builds no request at all. Defaults to `true`.
    fn is_active(&self) -> bool {
        true
    }

    /// Judge one claim.
    ///
    /// # Errors
    /// A [`FallbackReason`] when no verdict could be obtained.
    fn judge(&self, request: &SupportRequest) -> Result<SupportVerdict, FallbackReason>;
}

/// The deterministic default judge: the option is off. Builds nothing, judges
/// nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct NotJudged;

impl SupportJudge for NotJudged {
    fn is_active(&self) -> bool {
        false
    }

    fn judge(&self, _request: &SupportRequest) -> Result<SupportVerdict, FallbackReason> {
        Err(FallbackReason::FlagOff)
    }
}

/// Adapter from a host-supplied transport closure to a [`SupportJudge`].
///
/// The closure receives the System One request body and returns the response
/// body, or the [`FallbackReason`] describing the transport failure. The
/// library never opens a connection itself.
///
/// ```
/// use diagram_ir::semantic_review::{
///     review_claims, Claim, Evidence, FallbackReason, JsonJudge, Scope,
/// };
///
/// let judge = JsonJudge::new("jev-latest", |_body| Err(FallbackReason::Timeout));
/// let scope = Scope::new("d", "flowchart", vec![]);
/// let pair = (Claim::edge("e", "A", "B", "", true), Evidence::new("doc", "A calls B."));
/// let review = review_claims(&judge, &scope, vec![pair]);
/// assert_eq!(
///     review.items()[0].judgement().fallback(),
///     Some(&FallbackReason::Timeout)
/// );
/// ```
pub struct JsonJudge<F> {
    model: String,
    transport: F,
}

impl<F> JsonJudge<F>
where
    F: Fn(&Value) -> Result<Value, FallbackReason>,
{
    /// A judge that sends bodies for `model` through `transport`.
    pub fn new(model: impl Into<String>, transport: F) -> Self {
        Self {
            model: model.into(),
            transport,
        }
    }
}

impl<F> SupportJudge for JsonJudge<F>
where
    F: Fn(&Value) -> Result<Value, FallbackReason>,
{
    fn judge(&self, request: &SupportRequest) -> Result<SupportVerdict, FallbackReason> {
        let body = (self.transport)(&request.to_system_one(&self.model))?;
        request.parse_response(&body)
    }
}

/// The outcome for one claim: a verdict, or the reason there is none.
#[derive(Clone, Debug, PartialEq)]
pub enum Judgement {
    /// No verdict; the claim is as it was before the review existed.
    NotJudged(FallbackReason),
    /// A verdict (an annotation; see [`ReviewPolicy`] for when it is flagged).
    Judged(SupportVerdict),
}

impl Judgement {
    /// The verdict, if any.
    #[must_use]
    pub fn verdict(&self) -> Option<&SupportVerdict> {
        match self {
            Self::Judged(v) => Some(v),
            Self::NotJudged(_) => None,
        }
    }

    /// The fallback reason, if not judged.
    #[must_use]
    pub fn fallback(&self) -> Option<&FallbackReason> {
        match self {
            Self::NotJudged(r) => Some(r),
            Self::Judged(_) => None,
        }
    }
}

/// One reviewed claim.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaimReview {
    claim: Claim,
    evidence: Evidence,
    judgement: Judgement,
}

impl ClaimReview {
    /// The claim reviewed.
    #[must_use]
    pub fn claim(&self) -> &Claim {
        &self.claim
    }

    /// The evidence it was reviewed against.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }

    /// The judgement.
    #[must_use]
    pub fn judgement(&self) -> &Judgement {
        &self.judgement
    }
}

/// Thresholds for turning verdicts into flags.
///
/// The default flags **nothing**: an acceptance threshold must be fitted on
/// labelled data (for example seeded reversed-edge and mislabelling cases)
/// before findings are surfaced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReviewPolicy {
    /// Least `support` confidence at which a verdict is kept. `None` (default):
    /// verdicts are recorded but never flagged. Below it a claim becomes
    /// [`Judgement::NotJudged`] with [`FallbackReason::LowConfidence`].
    pub min_confidence: Option<f64>,
    /// A `noul` probability below this marks that obligation unsupported.
    pub unsupported_below: f64,
    /// An overall `support` score below this marks the claim unsupported.
    pub support_below: f64,
}

impl Default for ReviewPolicy {
    fn default() -> Self {
        Self {
            min_confidence: None,
            unsupported_below: 0.5,
            support_below: 1.5,
        }
    }
}

/// Which aspect of a claim a [`Finding`] is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aspect {
    /// Edge direction.
    Direction,
    /// Edge label.
    Label,
    /// Node grouping.
    Grouping,
    /// Overall support score.
    Support,
}

/// A flagged claim: the evidence does not support this aspect.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    /// IR id of the edge or member node.
    pub claim_id: String,
    /// The aspect that failed.
    pub aspect: Aspect,
    /// The probability or score that failed its threshold.
    pub value: f64,
    /// Source of the evidence the claim was judged against.
    pub evidence_source: String,
}

/// The result of a review run.
#[derive(Clone, Debug, PartialEq)]
pub struct Review {
    items: Vec<ClaimReview>,
}

impl Review {
    /// Every reviewed claim, in input order.
    #[must_use]
    pub fn items(&self) -> &[ClaimReview] {
        &self.items
    }

    /// The question-set version these results were produced with.
    #[must_use]
    pub fn question_set(&self) -> &'static str {
        QUESTION_SET
    }

    /// Findings under `policy`. Empty when `policy.min_confidence` is `None`,
    /// and for every claim that was not judged or fell below the confidence
    /// threshold.
    #[must_use]
    pub fn flagged(&self, policy: &ReviewPolicy) -> Vec<Finding> {
        let Some(min) = policy.min_confidence else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for it in &self.items {
            let Judgement::Judged(v) = &it.judgement else {
                continue;
            };
            if v.confidence < min {
                continue;
            }
            let mk = |aspect, value| Finding {
                claim_id: it.claim.id().to_owned(),
                aspect,
                value,
                evidence_source: it.evidence.source.clone(),
            };
            for (aspect, p) in [
                (Aspect::Direction, v.direction),
                (Aspect::Label, v.label),
                (Aspect::Grouping, v.grouping),
            ] {
                if let Some(p) = p.filter(|p| *p < policy.unsupported_below) {
                    out.push(mk(aspect, p));
                }
            }
            if v.support < policy.support_below {
                out.push(mk(Aspect::Support, v.support));
            }
        }
        out
    }
}

/// Review `claims` with `judge`.
///
/// Fail-open by construction: a judge that is inactive, errs, times out or
/// returns something malformed yields [`Judgement::NotJudged`] for that claim,
/// and the rest of the review continues. With an inactive judge no request is
/// built.
pub fn review_claims(
    judge: &dyn SupportJudge,
    scope: &Scope,
    claims: Vec<(Claim, Evidence)>,
) -> Review {
    review_claims_with(judge, scope, claims, &ReviewPolicy::default())
}

/// As [`review_claims`], applying `policy.min_confidence` to each verdict.
pub fn review_claims_with(
    judge: &dyn SupportJudge,
    scope: &Scope,
    claims: Vec<(Claim, Evidence)>,
    policy: &ReviewPolicy,
) -> Review {
    let active = judge.is_active();
    let items = claims
        .into_iter()
        .map(|(claim, evidence)| {
            let judgement = if active {
                let req = SupportRequest::new(scope, claim.clone(), evidence.clone());
                match judge.judge(&req) {
                    Ok(v) if policy.min_confidence.is_some_and(|m| v.confidence < m) => {
                        Judgement::NotJudged(FallbackReason::LowConfidence)
                    }
                    Ok(v) => Judgement::Judged(v),
                    Err(r) => Judgement::NotJudged(r),
                }
            } else {
                Judgement::NotJudged(FallbackReason::FlagOff)
            };
            ClaimReview {
                claim,
                evidence,
                judgement,
            }
        })
        .collect();
    Review { items }
}
