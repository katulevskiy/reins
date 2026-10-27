DROP TABLE reins_org_webhook_deliveries;
DROP TABLE reins_org_rollouts;
DROP TABLE reins_org_simulations;
DROP TABLE reins_org_policy_repo;
ALTER TABLE reins_org_policies DROP COLUMN rollout_plan;
ALTER TABLE reins_org_policies DROP COLUMN source_ref;
ALTER TABLE reins_org_requests DROP COLUMN rollout_seq;
ALTER TABLE reins_org_requests DROP COLUMN rollout_id;
ALTER TABLE reins_org_requests DROP COLUMN analysis;
