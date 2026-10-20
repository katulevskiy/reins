use proptest::prelude::*;
use reins_policy::authorization::*;
use reins_proto::authorization::*;

const NOW: i64 = 100;
const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const KEYS: &[AttributeKey] = &[
    AttributeKey::Repository,
    AttributeKey::Recipients,
    AttributeKey::CostMicrounits,
    AttributeKey::CostCurrency,
    AttributeKey::Branch,
    AttributeKey::NetworkRestricted,
];

fn operation() -> OperationId {
    OperationId {
        service: "desktop".into(),
        capability: "command".into(),
        action: "run".into(),
    }
}

fn known(key: AttributeKey, value: AttributeValue) -> AttributeFact {
    AttributeFact {
        key,
        fact: Fact::Known {
            value,
            evidence: Evidence {
                id: format!("verified-{key:?}"),
                source: EvidenceSource::Connector,
                observed_at: 90,
                valid_until: 110,
            },
        },
    }
}

fn action() -> ActionContext {
    ActionContext {
        v: AUTHORIZATION_VERSION,
        request_id: "request-1".into(),
        request_digest: DIGEST.into(),
        actor: ActorContext {
            principal_id: "employee".into(),
            tenant_id: Some("corp".into()),
            connection_id: "agent".into(),
            device_id: Some("computer".into()),
            executor_principal_id: "account-owner".into(),
            executor_device_id: "phone".into(),
        },
        operation: operation(),
        resource: ResourceContext {
            account_id: "account-1".into(),
            resource_id: Some("resource-1".into()),
        },
        facts: vec![known(AttributeKey::Repository, AttributeValue::String("repo-1".into()))],
    }
}

fn policy(id: &str, effect: PolicyEffect) -> PolicyRecord {
    PolicyRecord {
        v: AUTHORIZATION_VERSION,
        id: id.into(),
        revision: 1,
        issuer: PolicyIssuer {
            principal_id: "admin".into(),
            tenant_id: Some("corp".into()),
        },
        kind: if matches!(effect, PolicyEffect::Deny | PolicyEffect::RequireReview { .. }) {
            PolicyKind::Mandatory
        } else {
            PolicyKind::Permission
        },
        lifecycle: PolicyLifecycle::Active,
        valid_from: 80,
        expires_at: Some(120),
        scope: PolicyScope {
            connection_ids: vec!["agent".into()],
            account_ids: vec!["account-1".into()],
            operations: vec![operation()],
            ..PolicyScope::default()
        },
        conditions: vec![],
        effect,
        parent_policy_ids: vec![],
        limits: UsageLimits::default(),
    }
}

fn reviewer(role: &str, quorum: u32) -> ReviewRequirement {
    ReviewRequirement {
        role_id: role.into(),
        quorum,
        distinct_humans_group: Some("publication".into()),
        exclude_requester: true,
    }
}

fn run(action: &ActionContext, policies: &[PolicyRecord], baseline: BaselineDecision) -> AuthorizationDecision {
    run_at(action, policies, baseline, NOW, KEYS, &[])
}

fn run_at(
    action: &ActionContext,
    policies: &[PolicyRecord],
    baseline: BaselineDecision,
    now: i64,
    keys: &[AttributeKey],
    usage: &[UsageSnapshot],
) -> AuthorizationDecision {
    let ids: Vec<_> = policies.iter().map(|p| p.id.clone()).collect();
    let authority = AuthorityBindings {
        principal_id: "employee",
        tenant_id: Some("corp"),
        connection_id: "agent",
        device_id: Some("computer"),
        executor_principal_id: "account-owner",
        executor_device_id: "phone",
        account_id: "account-1",
        resource_id: Some("resource-1"),
        request_digest: DIGEST,
        account_access: true,
        verified_attribute_keys: keys,
        authorized_policy_ids: &ids,
    };
    evaluate(&EvaluationInput {
        action,
        policies,
        authority: &authority,
        now,
        usage,
        baseline,
    })
}

fn repo_condition(repo: &str) -> Condition {
    Condition::StringEquals {
        key: AttributeKey::Repository,
        value: repo.into(),
    }
}

fn assert_no_permit(decision: &AuthorizationDecision) {
    assert!(!matches!(decision.outcome, DecisionOutcome::Permit | DecisionOutcome::EligibleForAuto), "{decision:?}");
}

#[test]
fn legacy_baselines_are_preserved_without_new_policy() {
    for (baseline, expected) in [
        (BaselineDecision::Permit, DecisionOutcome::Permit),
        (BaselineDecision::Review, DecisionOutcome::Review),
        (BaselineDecision::Deny, DecisionOutcome::Deny),
    ] {
        assert_eq!(run(&action(), &[], baseline).outcome, expected);
    }
}

#[test]
fn deny_and_all_review_requirements_dominate_legacy_and_explicit_permissions() {
    let allow = policy("allow", PolicyEffect::Permit);
    let owner = policy(
        "owner-review",
        PolicyEffect::RequireReview {
            requirements: vec![reviewer("owner", 1)],
        },
    );
    let legal = policy(
        "legal-review",
        PolicyEffect::RequireReview {
            requirements: vec![reviewer("legal", 2)],
        },
    );
    let review = run(&action(), &[allow.clone(), owner.clone(), legal.clone()], BaselineDecision::Permit);
    assert_eq!(review.outcome, DecisionOutcome::Review);
    assert_eq!(review.required_reviews, vec![reviewer("legal", 2), reviewer("owner", 1)]);
    assert!(review.authorization_path.is_empty());
    let denied = run(&action(), &[allow, owner, legal, policy("deny", PolicyEffect::Deny)], BaselineDecision::Permit);
    assert_eq!(denied.outcome, DecisionOutcome::Deny);
    assert_eq!(denied.required_reviews.len(), 2);
}

#[test]
fn explicit_permission_is_immediate_and_auto_eligibility_never_satisfies_review() {
    assert_eq!(
        run(&action(), &[policy("allow", PolicyEffect::Permit)], BaselineDecision::Review).outcome,
        DecisionOutcome::Permit
    );
    let auto = policy("auto", PolicyEffect::EligibleForAuto);
    assert_eq!(
        run(&action(), std::slice::from_ref(&auto), BaselineDecision::Review).outcome,
        DecisionOutcome::EligibleForAuto
    );
    assert_eq!(
        run(
            &action(),
            &[
                auto,
                policy(
                    "review",
                    PolicyEffect::RequireReview {
                        requirements: vec![reviewer("owner", 1)]
                    }
                )
            ],
            BaselineDecision::Permit
        )
        .outcome,
        DecisionOutcome::Review
    );
}

#[test]
fn fresh_verified_facts_are_required_and_claimed_provenance_is_not_authority() {
    let mut allow = policy("allow", PolicyEffect::Permit);
    allow.conditions.push(repo_condition("repo-1"));
    assert_eq!(run(&action(), std::slice::from_ref(&allow), BaselineDecision::Review).outcome, DecisionOutcome::Permit);
    assert_no_permit(&run_at(&action(), std::slice::from_ref(&allow), BaselineDecision::Permit, NOW, &[], &[]));
    for source in [EvidenceSource::Agent, EvidenceSource::Organization] {
        let mut request = action();
        if let Fact::Known {
            evidence,
            ..
        } = &mut request.facts[0].fact
        {
            evidence.source = source;
        }
        let keys = if source == EvidenceSource::Agent {
            KEYS
        } else {
            &[]
        };
        assert_no_permit(&run_at(&request, std::slice::from_ref(&allow), BaselineDecision::Permit, NOW, keys, &[]));
    }
    for now in [89, 110, 111] {
        assert_no_permit(&run_at(&action(), std::slice::from_ref(&allow), BaselineDecision::Permit, now, KEYS, &[]));
    }
    let mut unknown = action();
    unknown.facts[0].fact = Fact::Unknown {
        reason: UnknownReason::Unresolved,
    };
    assert_no_permit(&run(&unknown, &[allow], BaselineDecision::Permit));
}

#[test]
fn unresolved_mandatory_guards_hold_even_with_other_valid_permission() {
    let mut deny = policy("corporate-repo", PolicyEffect::Deny);
    deny.conditions.push(repo_condition("company"));
    let mut request = action();
    request.facts.clear();
    let rules = [deny.clone(), policy("allow", PolicyEffect::Permit)];
    assert_eq!(run(&request, &rules, BaselineDecision::Permit).outcome, DecisionOutcome::Hold);
    assert_eq!(run_at(&action(), &rules, BaselineDecision::Permit, NOW, &[], &[]).outcome, DecisionOutcome::Hold);
    // A proven false conjunct makes the restriction irrelevant even if another conjunct is unknown.
    deny.conditions.push(Condition::StringEquals {
        key: AttributeKey::Branch,
        value: "main".into(),
    });
    assert_eq!(
        run(&action(), &[deny, policy("allow", PolicyEffect::Permit)], BaselineDecision::Review).outcome,
        DecisionOutcome::Permit
    );
}

#[test]
fn identity_and_exact_payload_binding_cover_beneficiary_agent_executor_and_account() {
    let mut requests = vec![];
    let mut a = action();
    a.actor.principal_id = "other".into();
    requests.push(a);
    let mut a = action();
    a.actor.tenant_id = None;
    requests.push(a);
    let mut a = action();
    a.actor.tenant_id = Some("other-corp".into());
    requests.push(a);
    let mut a = action();
    a.actor.connection_id = "new-agent".into();
    requests.push(a);
    let mut a = action();
    a.actor.device_id = Some("other-computer".into());
    requests.push(a);
    let mut a = action();
    a.actor.executor_principal_id = "other-owner".into();
    requests.push(a);
    let mut a = action();
    a.actor.executor_device_id = "other-phone".into();
    requests.push(a);
    let mut a = action();
    a.resource.account_id = "other-account".into();
    requests.push(a);
    let mut a = action();
    a.resource.resource_id = Some("other-resource".into());
    requests.push(a);
    let mut a = action();
    a.request_digest = "b".repeat(64);
    requests.push(a);
    for request in requests {
        assert_eq!(
            run(&request, &[policy("allow", PolicyEffect::Permit)], BaselineDecision::Permit).outcome,
            DecisionOutcome::Deny
        );
    }
}

#[test]
fn cloud_agents_do_not_invent_computer_identity_and_device_scopes_hold() {
    let mut a = action();
    a.actor.device_id = None;
    let ids = ["restriction".into()];
    let authority = AuthorityBindings {
        principal_id: "employee",
        tenant_id: Some("corp"),
        connection_id: "agent",
        device_id: None,
        executor_principal_id: "account-owner",
        executor_device_id: "phone",
        account_id: "account-1",
        resource_id: Some("resource-1"),
        request_digest: DIGEST,
        account_access: true,
        verified_attribute_keys: KEYS,
        authorized_policy_ids: &ids,
    };
    let input = |policies| EvaluationInput {
        action: &a,
        policies,
        authority: &authority,
        now: NOW,
        usage: &[],
        baseline: BaselineDecision::Permit,
    };
    assert_eq!(evaluate(&input(&[])).outcome, DecisionOutcome::Permit);
    let mut deny = policy("restriction", PolicyEffect::Deny);
    deny.scope.device_ids.push("computer".into());
    assert_eq!(evaluate(&input(&[deny])).outcome, DecisionOutcome::Hold);
}

#[test]
fn wrong_policy_ownership_account_agent_and_future_capability_cannot_authorize() {
    let mut variations = vec![];
    let mut p = policy("wrong", PolicyEffect::Permit);
    p.issuer.tenant_id = Some("other-corp".into());
    variations.push(p);
    let mut p = policy("wrong", PolicyEffect::Permit);
    p.issuer.tenant_id = None;
    variations.push(p);
    let mut p = policy("wrong", PolicyEffect::Permit);
    p.scope.account_ids = vec!["other-account".into()];
    variations.push(p);
    let mut p = policy("wrong", PolicyEffect::Permit);
    p.scope.connection_ids = vec!["new-agent".into()];
    variations.push(p);
    let mut p = policy("wrong", PolicyEffect::Permit);
    p.scope.operations[0].action = "run_privileged".into();
    variations.push(p);
    let mut p = policy("wrong", PolicyEffect::Permit);
    p.scope.operations[0].capability = "command_new".into();
    variations.push(p);
    for p in variations {
        assert_no_permit(&run(&action(), &[p], BaselineDecision::Review));
    }
}

#[test]
fn recipient_sets_are_conjunctive_and_every_recipient_is_covered() {
    let mut p = policy("friends", PolicyEffect::Permit);
    p.conditions = vec![
        repo_condition("repo-1"),
        Condition::SetWithin {
            key: AttributeKey::Recipients,
            values: vec!["friend@example.com".into()],
            require_nonempty: true,
        },
    ];
    let mut a = action();
    for (recipients, expected) in [
        (vec!["friend@example.com".into()], DecisionOutcome::Permit),
        (vec!["friend@example.com".into(), "boss@corp.com".into()], DecisionOutcome::Review),
        (vec![], DecisionOutcome::Review),
    ] {
        a.facts.truncate(1);
        a.facts.push(known(AttributeKey::Recipients, AttributeValue::StringSet(recipients)));
        assert_eq!(run(&a, std::slice::from_ref(&p), BaselineDecision::Permit).outcome, expected);
    }
    p.conditions.push(Condition::BoolEquals {
        key: AttributeKey::NetworkRestricted,
        value: true,
    });
    assert_no_permit(&run(&a, &[p], BaselineDecision::Permit));
}

#[test]
fn paused_revoked_expired_permissions_do_not_resurrect_legacy_grants() {
    for lifecycle in [PolicyLifecycle::Paused, PolicyLifecycle::Revoked] {
        let mut p = policy("limited", PolicyEffect::Permit);
        p.lifecycle = lifecycle;
        assert_eq!(run(&action(), std::slice::from_ref(&p), BaselineDecision::Permit).outcome, DecisionOutcome::Review);
        // Independent live permissions remain OR sources; revoking one is not an implicit global deny.
        assert_eq!(
            run(&action(), &[p, policy("independent", PolicyEffect::Permit)], BaselineDecision::Review).outcome,
            DecisionOutcome::Permit
        );
    }
    let p = policy("expiring", PolicyEffect::Permit);
    for now in [79, 120, 121] {
        assert_no_permit(&run_at(&action(), std::slice::from_ref(&p), BaselineDecision::Permit, now, KEYS, &[]));
    }
    assert_eq!(run_at(&action(), &[p], BaselineDecision::Review, 80, KEYS, &[]).outcome, DecisionOutcome::Permit);
}

fn delegation() -> (PolicyRecord, PolicyRecord) {
    let mut parent = policy("authority", PolicyEffect::DelegationAuthority);
    parent.conditions.push(repo_condition("repo-1"));
    let mut child = policy("session", PolicyEffect::Permit);
    child.kind = PolicyKind::Delegation;
    child.parent_policy_ids.push(parent.id.clone());
    (parent, child)
}

fn personal_decision(policies: &[PolicyRecord], baseline: BaselineDecision) -> AuthorizationDecision {
    personal_decision_with_device(policies, baseline, Some("computer"))
}

fn personal_decision_with_device(
    policies: &[PolicyRecord],
    baseline: BaselineDecision,
    device: Option<&str>,
) -> AuthorizationDecision {
    let mut action = action();
    action.actor.tenant_id = None;
    action.actor.device_id = device.map(str::to_owned);
    let mut policies = policies.to_vec();
    for policy in &mut policies {
        policy.issuer.tenant_id = None;
        policy.issuer.principal_id.clone_from(&action.actor.executor_principal_id);
    }
    let ids = policies.iter().map(|policy| policy.id.clone()).collect::<Vec<_>>();
    let authority = AuthorityBindings {
        principal_id: &action.actor.principal_id,
        tenant_id: None,
        connection_id: &action.actor.connection_id,
        device_id: action.actor.device_id.as_deref(),
        executor_principal_id: &action.actor.executor_principal_id,
        executor_device_id: &action.actor.executor_device_id,
        account_id: &action.resource.account_id,
        resource_id: action.resource.resource_id.as_deref(),
        request_digest: &action.request_digest,
        account_access: true,
        verified_attribute_keys: KEYS,
        authorized_policy_ids: &ids,
    };
    evaluate(&EvaluationInput {
        action: &action,
        policies: &policies,
        authority: &authority,
        now: NOW,
        usage: &[],
        baseline,
    })
}

fn ask_default() -> PolicyRecord {
    let mut policy = policy("ask-default", PolicyEffect::ReviewByDefault);
    policy.kind = PolicyKind::AccountDefault;
    policy
}

#[test]
fn personal_ask_default_suppresses_legacy_but_explicit_delegation_can_authorize() {
    let ask = ask_default();
    let reviewed = personal_decision(std::slice::from_ref(&ask), BaselineDecision::Permit);
    assert_eq!(reviewed.outcome, DecisionOutcome::Review);
    assert!(reviewed.required_reviews.is_empty(), "a fallback imposes no hard review obligation");
    assert!(reviewed.reasons.iter().any(|r| r.code == ReasonCode::ReviewByDefault));
    let (parent, child) = delegation();
    for policies in [vec![ask.clone(), parent.clone(), child.clone()], vec![child.clone(), ask.clone(), parent.clone()]]
    {
        let allowed = personal_decision(&policies, BaselineDecision::Review);
        assert_eq!(allowed.outcome, DecisionOutcome::Permit);
        assert_eq!(allowed.authorization_path.len(), 2);
    }
    let mut expired = child;
    expired.expires_at = Some(NOW);
    assert_eq!(personal_decision(&[ask, parent, expired], BaselineDecision::Permit).outcome, DecisionOutcome::Review);
}

#[test]
fn hard_reviews_and_denies_still_override_soft_defaults_and_delegations() {
    let (parent, child) = delegation();
    for kind in [PolicyKind::Mandatory, PolicyKind::AccountDefault, PolicyKind::Permission] {
        let mut hard = policy(
            "always-ask",
            PolicyEffect::RequireReview {
                requirements: vec![reviewer("owner", 1)],
            },
        );
        hard.kind = kind;
        let review = personal_decision(&[ask_default(), parent.clone(), child.clone(), hard], BaselineDecision::Permit);
        assert_eq!(review.outcome, DecisionOutcome::Review);
        assert_eq!(review.required_reviews, vec![reviewer("owner", 1)]);
    }
    let deny = policy("never", PolicyEffect::Deny);
    assert_eq!(
        personal_decision(&[ask_default(), parent, child, deny], BaselineDecision::Permit).outcome,
        DecisionOutcome::Deny
    );
}

#[test]
fn unknown_soft_default_never_vetoes_a_verified_permission_or_releases_legacy() {
    for unknown_scope in [false, true] {
        let mut ask = ask_default();
        if unknown_scope {
            ask.scope.device_ids = vec!["computer".into()];
        } else {
            ask.conditions.push(Condition::StringEquals {
                key: AttributeKey::Workspace,
                value: "workspace".into(),
            });
        }
        assert_eq!(
            personal_decision_with_device(std::slice::from_ref(&ask), BaselineDecision::Permit, None).outcome,
            DecisionOutcome::Review
        );
        let (parent, child) = delegation();
        assert_eq!(
            personal_decision_with_device(&[ask, parent, child], BaselineDecision::Permit, None).outcome,
            DecisionOutcome::Permit
        );
    }
    let mut ask = ask_default();
    ask.conditions.push(repo_condition("another"));
    assert_eq!(
        personal_decision(&[ask], BaselineDecision::Permit).outcome,
        DecisionOutcome::Permit,
        "nonmatching fallback does not alter another scope"
    );
}

#[test]
fn delegation_authority_does_not_execute_and_every_ancestor_remains_a_bound() {
    let (parent, child) = delegation();
    assert_no_permit(&run(&action(), std::slice::from_ref(&parent), BaselineDecision::Review));
    let decision = run(&action(), &[parent.clone(), child.clone()], BaselineDecision::Review);
    assert_eq!(decision.outcome, DecisionOutcome::Permit);
    assert_eq!(
        decision.authorization_path.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["authority", "session"]
    );
    for which in 0..2 {
        for lifecycle in [PolicyLifecycle::Paused, PolicyLifecycle::Revoked] {
            let mut rules = [parent.clone(), child.clone()];
            rules[which].lifecycle = lifecycle;
            assert_no_permit(&run(&action(), &rules, BaselineDecision::Permit));
        }
    }
    let mut wrong_repo = action();
    wrong_repo.facts[0] = known(AttributeKey::Repository, AttributeValue::String("another".into()));
    assert_no_permit(&run(&wrong_repo, &[parent.clone(), child.clone()], BaselineDecision::Permit));
    let mut wrong_scope = parent.clone();
    wrong_scope.scope.account_ids = vec!["another".into()];
    assert_no_permit(&run(&action(), &[wrong_scope, child.clone()], BaselineDecision::Permit));
    let mut auto_parent = parent;
    auto_parent.effect = PolicyEffect::EligibleForAuto;
    assert_eq!(
        run(&action(), &[auto_parent, child], BaselineDecision::Review).outcome,
        DecisionOutcome::EligibleForAuto
    );
}

#[test]
fn malformed_and_cyclic_delegations_hold_before_legacy_authority() {
    let (parent, mut child) = delegation();
    assert_eq!(run(&action(), std::slice::from_ref(&child), BaselineDecision::Permit).outcome, DecisionOutcome::Hold);
    let mut parent = parent;
    parent.kind = PolicyKind::Delegation;
    parent.effect = PolicyEffect::Permit;
    parent.parent_policy_ids = vec![child.id.clone()];
    assert_eq!(run(&action(), &[parent, child.clone()], BaselineDecision::Permit).outcome, DecisionOutcome::Hold);
    child.parent_policy_ids = vec![child.id.clone()];
    assert_eq!(run(&action(), &[child], BaselineDecision::Permit).outcome, DecisionOutcome::Hold);
}

fn usage(id: &str) -> UsageSnapshot {
    UsageSnapshot {
        policy_id: id.into(),
        revision: 1,
        committed_uses: 0,
        reserved_uses: 0,
        committed_cost_microunits: 0,
        reserved_cost_microunits: 0,
        cost_currency: None,
    }
}

#[test]
fn limits_include_pending_reservations_parent_budgets_and_overflow() {
    let (mut parent, child) = delegation();
    parent.limits.max_uses = Some(2);
    let rules = [parent, child];
    assert_no_permit(&run(&action(), &rules, BaselineDecision::Permit));
    let mut u = usage("authority");
    u.committed_uses = 1;
    assert_eq!(
        run_at(&action(), &rules, BaselineDecision::Review, NOW, KEYS, std::slice::from_ref(&u)).outcome,
        DecisionOutcome::Permit
    );
    u.reserved_uses = 1;
    assert_no_permit(&run_at(&action(), &rules, BaselineDecision::Permit, NOW, KEYS, std::slice::from_ref(&u)));
    u.committed_uses = u64::MAX;
    assert_no_permit(&run_at(&action(), &rules, BaselineDecision::Permit, NOW, KEYS, std::slice::from_ref(&u)));
    u.committed_uses = 0;
    u.reserved_uses = 0;
    u.revision = 2;
    assert_no_permit(&run_at(&action(), &rules, BaselineDecision::Permit, NOW, KEYS, &[u]));
}

#[test]
fn monetary_limits_require_verified_matching_currency_and_include_request_cost() {
    let mut a = action();
    a.facts.push(known(AttributeKey::CostMicrounits, AttributeValue::U64(5)));
    a.facts.push(known(AttributeKey::CostCurrency, AttributeValue::Currency(Currency::Usd)));
    let mut p = policy("budget", PolicyEffect::Permit);
    p.limits = UsageLimits {
        max_uses: None,
        max_cost_microunits: Some(10),
        cost_currency: Some(Currency::Usd),
    };
    let mut u = usage("budget");
    u.cost_currency = Some(Currency::Usd);
    u.committed_cost_microunits = 3;
    u.reserved_cost_microunits = 2;
    assert_eq!(
        run_at(&a, std::slice::from_ref(&p), BaselineDecision::Review, NOW, KEYS, std::slice::from_ref(&u)).outcome,
        DecisionOutcome::Permit
    );
    u.reserved_cost_microunits = 3;
    assert_no_permit(&run_at(
        &a,
        std::slice::from_ref(&p),
        BaselineDecision::Permit,
        NOW,
        KEYS,
        std::slice::from_ref(&u),
    ));
    u.reserved_cost_microunits = 0;
    u.cost_currency = Some(Currency::Eur);
    assert_no_permit(&run_at(
        &a,
        std::slice::from_ref(&p),
        BaselineDecision::Permit,
        NOW,
        KEYS,
        std::slice::from_ref(&u),
    ));
    u.cost_currency = Some(Currency::Usd);
    a.facts[2] = known(AttributeKey::CostCurrency, AttributeValue::Currency(Currency::Eur));
    assert_no_permit(&run_at(&a, &[p], BaselineDecision::Permit, NOW, KEYS, &[u]));
}

#[test]
fn unverified_policy_unsupported_version_duplicates_and_oversized_snapshots_hold() {
    let a = action();
    let p = policy("allow", PolicyEffect::Permit);
    let authority = AuthorityBindings {
        principal_id: "employee",
        tenant_id: Some("corp"),
        connection_id: "agent",
        device_id: Some("computer"),
        executor_principal_id: "account-owner",
        executor_device_id: "phone",
        account_id: "account-1",
        resource_id: Some("resource-1"),
        request_digest: DIGEST,
        account_access: true,
        verified_attribute_keys: KEYS,
        authorized_policy_ids: &[],
    };
    let untrusted = evaluate(&EvaluationInput {
        action: &a,
        policies: std::slice::from_ref(&p),
        authority: &authority,
        now: NOW,
        usage: &[],
        baseline: BaselineDecision::Permit,
    });
    assert_eq!(untrusted.outcome, DecisionOutcome::Hold);
    let mut unknown = p.clone();
    unknown.v = 2;
    assert_eq!(run(&a, &[p.clone(), unknown], BaselineDecision::Permit).outcome, DecisionOutcome::Hold);
    assert_eq!(run(&a, &[p.clone(), p.clone()], BaselineDecision::Permit).outcome, DecisionOutcome::Hold);
    assert_eq!(
        run(&a, &vec![p; MAX_AUTHORIZATION_POLICIES + 1], BaselineDecision::Permit).outcome,
        DecisionOutcome::Hold
    );
    let mut no_access = authority;
    no_access.account_access = false;
    assert_eq!(
        evaluate(&EvaluationInput {
            action: &a,
            policies: &[],
            authority: &no_access,
            now: NOW,
            usage: &[],
            baseline: BaselineDecision::Permit
        })
        .outcome,
        DecisionOutcome::Deny
    );
}

proptest! {
    #[test]
    fn policy_permutations_do_not_change_decisions(order in prop::collection::vec(any::<u8>(), 5), include_deny in any::<bool>()) {
        let mut rules = vec![policy("allow", PolicyEffect::Permit), policy("auto", PolicyEffect::EligibleForAuto),
            policy("owner", PolicyEffect::RequireReview { requirements: vec![reviewer("owner", 1)] }),
            policy("legal", PolicyEffect::RequireReview { requirements: vec![reviewer("legal", 2)] })];
        if include_deny { rules.push(policy("deny", PolicyEffect::Deny)); }
        let expected = run(&action(), &rules, BaselineDecision::Permit);
        let mut keyed: Vec<_> = rules.into_iter().enumerate().map(|(i, p)| (order[i], p)).collect();
        keyed.sort_by_key(|(key, _)| *key);
        let shuffled: Vec<_> = keyed.into_iter().map(|(_, p)| p).collect();
        prop_assert_eq!(run(&action(), &shuffled, BaselineDecision::Permit), expected);
    }

    #[test]
    fn adding_mandatory_restrictions_never_authorizes(allow in any::<bool>(), baseline_permit in any::<bool>(), deny in any::<bool>()) {
        let baseline = if baseline_permit { BaselineDecision::Permit } else { BaselineDecision::Review };
        let mut rules = if allow { vec![policy("allow", PolicyEffect::Permit)] } else { vec![] };
        rules.push(policy("restriction", if deny { PolicyEffect::Deny } else { PolicyEffect::RequireReview { requirements: vec![reviewer("owner", 1)] } }));
        prop_assert!(!matches!(run(&action(), &rules, baseline).outcome, DecisionOutcome::Permit | DecisionOutcome::EligibleForAuto));
    }

    #[test]
    fn every_recipient_must_be_in_the_selected_set(extra in "[a-z]{1,20}@example\\.com") {
        let mut a = action(); a.facts.push(known(AttributeKey::Recipients, AttributeValue::StringSet(vec!["friend@example.com".into(), extra.clone()])));
        let mut p = policy("friends", PolicyEffect::Permit);
        p.conditions.push(Condition::SetWithin { key: AttributeKey::Recipients, values: vec!["friend@example.com".into()], require_nonempty: true });
        prop_assume!(extra != "friend@example.com");
        prop_assert_ne!(run(&a, &[p], BaselineDecision::Permit).outcome, DecisionOutcome::Permit);
    }
}
