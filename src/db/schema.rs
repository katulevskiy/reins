table! {
    attachments (id) {
        id -> Text,
        cipher_uuid -> Text,
        file_name -> Text,
        file_size -> BigInt,
        akey -> Nullable<Text>,
    }
}

table! {
    ciphers (uuid) {
        uuid -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        user_uuid -> Nullable<Text>,
        organization_uuid -> Nullable<Text>,
        key -> Nullable<Text>,
        atype -> Integer,
        name -> Text,
        notes -> Nullable<Text>,
        fields -> Nullable<Text>,
        data -> Text,
        password_history -> Nullable<Text>,
        deleted_at -> Nullable<Timestamp>,
        reprompt -> Nullable<Integer>,
    }
}

table! {
    ciphers_collections (cipher_uuid, collection_uuid) {
        cipher_uuid -> Text,
        collection_uuid -> Text,
    }
}

table! {
    collections (uuid) {
        uuid -> Text,
        org_uuid -> Text,
        name -> Text,
        external_id -> Nullable<Text>,
    }
}

table! {
    devices (uuid, user_uuid) {
        uuid -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        user_uuid -> Text,
        name -> Text,
        atype -> Integer,
        push_uuid -> Nullable<Text>,
        push_token -> Nullable<Text>,
        refresh_token -> Text,
        twofactor_remember -> Nullable<Text>,
    }
}

table! {
    event (uuid) {
        uuid -> Text,
        event_type -> Integer,
        user_uuid -> Nullable<Text>,
        org_uuid -> Nullable<Text>,
        cipher_uuid -> Nullable<Text>,
        collection_uuid -> Nullable<Text>,
        group_uuid -> Nullable<Text>,
        org_user_uuid -> Nullable<Text>,
        act_user_uuid -> Nullable<Text>,
        device_type -> Nullable<Integer>,
        ip_address -> Nullable<Text>,
        event_date -> Timestamp,
        policy_uuid -> Nullable<Text>,
        provider_uuid -> Nullable<Text>,
        provider_user_uuid -> Nullable<Text>,
        provider_org_uuid -> Nullable<Text>,
    }
}

table! {
    favorites (user_uuid, cipher_uuid) {
        user_uuid -> Text,
        cipher_uuid -> Text,
    }
}

table! {
    folders (uuid) {
        uuid -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        user_uuid -> Text,
        name -> Text,
    }
}

table! {
    folders_ciphers (cipher_uuid, folder_uuid) {
        cipher_uuid -> Text,
        folder_uuid -> Text,
    }
}

table! {
    invitations (email) {
        email -> Text,
    }
}

table! {
    org_policies (uuid) {
        uuid -> Text,
        org_uuid -> Text,
        atype -> Integer,
        enabled -> Bool,
        data -> Text,
    }
}

table! {
    organizations (uuid) {
        uuid -> Text,
        name -> Text,
        billing_email -> Text,
        private_key -> Nullable<Text>,
        public_key -> Nullable<Text>,
    }
}

table! {
    sends (uuid) {
        uuid -> Text,
        user_uuid -> Nullable<Text>,
        organization_uuid -> Nullable<Text>,
        name -> Text,
        notes -> Nullable<Text>,
        atype -> Integer,
        data -> Text,
        akey -> Text,
        password_hash -> Nullable<Binary>,
        password_salt -> Nullable<Binary>,
        password_iter -> Nullable<Integer>,
        max_access_count -> Nullable<Integer>,
        access_count -> Integer,
        creation_date -> Timestamp,
        revision_date -> Timestamp,
        expiration_date -> Nullable<Timestamp>,
        deletion_date -> Timestamp,
        disabled -> Bool,
        hide_email -> Nullable<Bool>,
    }
}

table! {
    twofactor (uuid) {
        uuid -> Text,
        user_uuid -> Text,
        atype -> Integer,
        enabled -> Bool,
        data -> Text,
        last_used -> BigInt,
    }
}

table! {
    twofactor_incomplete (user_uuid, device_uuid) {
        user_uuid -> Text,
        device_uuid -> Text,
        device_name -> Text,
        device_type -> Integer,
        login_time -> Timestamp,
        ip_address -> Text,
    }
}

table! {
    twofactor_duo_ctx (state) {
        state -> Text,
        user_email -> Text,
        nonce -> Text,
        exp -> BigInt,
    }
}

table! {
    users (uuid) {
        uuid -> Text,
        enabled -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        verified_at -> Nullable<Timestamp>,
        last_verifying_at -> Nullable<Timestamp>,
        login_verify_count -> Integer,
        email -> Text,
        email_new -> Nullable<Text>,
        email_new_token -> Nullable<Text>,
        name -> Text,
        password_hash -> Binary,
        salt -> Binary,
        password_iterations -> Integer,
        password_hint -> Nullable<Text>,
        akey -> Text,
        private_key -> Nullable<Text>,
        public_key -> Nullable<Text>,
        totp_secret -> Nullable<Text>,
        totp_recover -> Nullable<Text>,
        security_stamp -> Text,
        stamp_exception -> Nullable<Text>,
        equivalent_domains -> Text,
        excluded_globals -> Text,
        client_kdf_type -> Integer,
        client_kdf_iter -> Integer,
        client_kdf_memory -> Nullable<Integer>,
        client_kdf_parallelism -> Nullable<Integer>,
        api_key -> Nullable<Text>,
        avatar_color -> Nullable<Text>,
        external_id -> Nullable<Text>,
    }
}

table! {
    users_collections (user_uuid, collection_uuid) {
        user_uuid -> Text,
        collection_uuid -> Text,
        read_only -> Bool,
        hide_passwords -> Bool,
        manage -> Bool,
    }
}

table! {
    users_organizations (uuid) {
        uuid -> Text,
        user_uuid -> Text,
        org_uuid -> Text,
        invited_by_email -> Nullable<Text>,
        access_all -> Bool,
        akey -> Text,
        status -> Integer,
        atype -> Integer,
        reset_password_key -> Nullable<Text>,
        external_id -> Nullable<Text>,
    }
}

table! {
    organization_api_key (uuid, org_uuid) {
        uuid -> Text,
        org_uuid -> Text,
        atype -> Integer,
        api_key -> Text,
        revision_date -> Timestamp,
    }
}

table! {
    sso_auth (state) {
        state -> Text,
        client_challenge -> Text,
        nonce -> Text,
        redirect_uri -> Text,
        code_response -> Nullable<Text>,
        code_response_error -> Nullable<Text>,
        auth_response -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        binding_hash -> Nullable<Text>,
    }
}

table! {
    sso_users (user_uuid) {
        user_uuid -> Text,
        identifier -> Text,
    }
}

table! {
    emergency_access (uuid) {
        uuid -> Text,
        grantor_uuid -> Text,
        grantee_uuid -> Nullable<Text>,
        email -> Nullable<Text>,
        key_encrypted -> Nullable<Text>,
        atype -> Integer,
        status -> Integer,
        wait_time_days -> Integer,
        recovery_initiated_at -> Nullable<Timestamp>,
        last_notification_at -> Nullable<Timestamp>,
        updated_at -> Timestamp,
        created_at -> Timestamp,
    }
}

table! {
    groups (uuid) {
        uuid -> Text,
        organizations_uuid -> Text,
        name -> Text,
        access_all -> Bool,
        external_id -> Nullable<Text>,
        creation_date -> Timestamp,
        revision_date -> Timestamp,
    }
}

table! {
    groups_users (groups_uuid, users_organizations_uuid) {
        groups_uuid -> Text,
        users_organizations_uuid -> Text,
    }
}

table! {
    collections_groups (collections_uuid, groups_uuid) {
        collections_uuid -> Text,
        groups_uuid -> Text,
        read_only -> Bool,
        hide_passwords -> Bool,
        manage -> Bool,
    }
}

table! {
    auth_requests  (uuid) {
        uuid -> Text,
        user_uuid -> Text,
        organization_uuid -> Nullable<Text>,
        request_device_identifier -> Text,
        device_type -> Integer,
        request_ip -> Text,
        response_device_id -> Nullable<Text>,
        access_code -> Text,
        public_key -> Text,
        enc_key -> Nullable<Text>,
        master_password_hash -> Nullable<Text>,
        approved -> Nullable<Bool>,
        creation_date -> Timestamp,
        response_date -> Nullable<Timestamp>,
        authentication_date -> Nullable<Timestamp>,
    }
}

table! {
    archives (user_uuid, cipher_uuid) {
        user_uuid -> Text,
        cipher_uuid -> Text,
        archived_at -> Timestamp,
    }
}

table! {
    reins_devices (user_uuid) {
        user_uuid -> Text,
        device_uuid -> Text,
        fcm_token -> Nullable<Text>,
        updated_at -> BigInt,
        key_hash -> Nullable<Text>,
    }
}

table! {
    reins_clients (client_id) {
        client_id -> Text,
        client_name -> Text,
        redirect_uris -> Text,
        created_at -> BigInt,
    }
}

table! {
    reins_jwt_clients (client_hash) {
        client_hash -> Text,
        first_seen_at -> BigInt,
    }
}

table! {
    reins_connections (uuid) {
        uuid -> Text,
        user_uuid -> Text,
        client_id -> Text,
        client_name -> Text,
        client_host -> Text,
        label -> Text,
        created_at -> BigInt,
        last_used_at -> Nullable<BigInt>,
        sealed_name -> Nullable<Text>,
    }
}

table! {
    reins_refresh_tokens (token_hash) {
        token_hash -> Text,
        connection_uuid -> Text,
        expires_at -> BigInt,
        client_auth -> Text,
    }
}

table! {
    reins_settings (name) {
        name -> Text,
        value -> Text,
    }
}

table! {
    reins_vault_passkeys (user_uuid, credential_hash) {
        user_uuid -> Text,
        credential_hash -> Text,
        credential_id -> Text,
        wrapped -> Text,
        name -> Text,
        created_at -> BigInt,
    }
}

table! {
    reins_device_signouts (user_uuid, device_uuid) {
        user_uuid -> Text,
        device_uuid -> Text,
        signed_out_at -> BigInt,
        by_device_name -> Text,
    }
}

table! {
    reins_sso_sessions (session_id) {
        session_id -> Text,
        user_uuid -> Text,
        device_uuid -> Text,
        created_at -> BigInt,
    }
}

table! {
    reins_members (user_uuid, device_uuid) {
        user_uuid -> Text,
        device_uuid -> Text,
        role -> Text,
        sign_pub -> Text,
        key_hash -> Nullable<Text>,
        fcm_token -> Nullable<Text>,
        last_counter -> BigInt,
        acked_seq -> BigInt,
        heartbeat -> Nullable<Text>,
        heartbeat_at -> Nullable<BigInt>,
        updated_at -> BigInt,
    }
}

table! {
    reins_rosters (user_uuid, epoch) {
        user_uuid -> Text,
        epoch -> BigInt,
        entry -> Text,
        entry_hash -> Text,
        recovery_wrap -> Nullable<Text>,
        bundle -> Nullable<Text>,
        created_at -> BigInt,
    }
}

table! {
    reins_ops (user_uuid, seq) {
        user_uuid -> Text,
        seq -> BigInt,
        author -> Text,
        sign_pub -> Text,
        counter -> BigInt,
        op -> Text,
        size -> Integer,
        created_at -> BigInt,
    }
}

table! {
    reins_checkpoints (user_uuid, seq) {
        user_uuid -> Text,
        seq -> BigInt,
        checkpoint -> Text,
        created_at -> BigInt,
    }
}

table! {
    reins_orgs (org_uuid) {
        org_uuid -> Text,
        tier -> Text,
        identity -> Text,
        workos_org_id -> Nullable<Text>,
        directory_id -> Nullable<Text>,
        github_org -> Nullable<Text>,
        two_person -> Bool,
        owner_recovery_at -> Nullable<BigInt>,
        created_by -> Text,
        created_at -> BigInt,
        updated_at -> BigInt,
        body_retention_days -> Integer,
    }
}

table! {
    reins_org_domains (org_uuid, domain) {
        org_uuid -> Text,
        domain -> Text,
        token -> Text,
        verified_at -> Nullable<BigInt>,
        checked_at -> Nullable<BigInt>,
        created_at -> BigInt,
    }
}

table! {
    reins_org_verified_domains (domain) {
        domain -> Text,
        org_uuid -> Text,
        verified_at -> BigInt,
    }
}

table! {
    reins_org_members (org_uuid, user_uuid) {
        org_uuid -> Text,
        user_uuid -> Text,
        membership_uuid -> Text,
        state -> Text,
        changed_at -> BigInt,
        changed_by -> Nullable<Text>,
    }
}

table! {
    reins_org_roles (org_uuid, subject_kind, subject_id, role) {
        org_uuid -> Text,
        subject_kind -> Text,
        subject_id -> Text,
        role -> Text,
        granted_at -> BigInt,
        granted_by -> Text,
    }
}

table! {
    reins_org_changes (uuid) {
        uuid -> Text,
        org_uuid -> Text,
        action -> Text,
        proposed_by -> Text,
        proposed_at -> BigInt,
        expires_at -> BigInt,
        state -> Text,
        decided_by -> Nullable<Text>,
        decided_at -> Nullable<BigInt>,
    }
}

table! {
    reins_org_devices (org_uuid, user_uuid, device_uuid) {
        org_uuid -> Text,
        user_uuid -> Text,
        device_uuid -> Text,
        key_algorithm -> Text,
        public_key -> Text,
        hardware -> Bool,
        attestations -> Text,
        posture -> Text,
        posture_at -> BigInt,
        enrolled_at -> BigInt,
        revoked_at -> Nullable<BigInt>,
        attestation_state -> Text,
        attestation_detail -> Nullable<Text>,
    }
}

table! {
    reins_org_events (org_uuid, seq) {
        org_uuid -> Text,
        seq -> BigInt,
        at -> BigInt,
        kind -> Text,
        actor -> Nullable<Text>,
        subject -> Nullable<Text>,
        detail -> Text,
        prev_hash -> Text,
        hash -> Text,
    }
}

table! {
    reins_tickets (id) {
        id -> Text,
        user_uuid -> Text,
        connection_uuid -> Text,
        connection_label -> Text,
        state -> Text,
        digest -> Text,
        execution -> Text,
        on_expiry -> Text,
        finish -> Text,
        created_at -> BigInt,
        deferred_at -> BigInt,
        deadline -> BigInt,
        window_secs -> BigInt,
        escalated -> Bool,
        approved_at -> Nullable<BigInt>,
        valid_secs -> BigInt,
        sealed_body -> Text,
        question -> Nullable<Text>,
        sealed_reply -> Nullable<Text>,
        note -> Nullable<Text>,
        result -> Nullable<Text>,
        stale -> Nullable<Text>,
        delivered -> Bool,
        updated_at -> BigInt,
    }
}

table! {
    reins_ticket_events (ticket_id, seq) {
        ticket_id -> Text,
        seq -> Integer,
        state -> Text,
        signed -> Bool,
        event -> Text,
        created_at -> BigInt,
    }
}

table! {
    reins_device_keys (user_uuid) {
        user_uuid -> Text,
        device_uuid -> Text,
        sign_pub -> Text,
        box_pub -> Text,
        updated_at -> BigInt,
    }
}

table! {
    reins_callbacks (connection_uuid) {
        connection_uuid -> Text,
        url -> Text,
        verified_at -> BigInt,
    }
}

table! {
    reins_org_keys (org_uuid, key_id) {
        org_uuid -> Text,
        key_id -> Text,
        purpose -> Text,
        public_key -> Text,
        sealed -> Text,
        created_at -> BigInt,
        retired_at -> Nullable<BigInt>,
    }
}

table! {
    reins_org_policies (org_uuid, revision) {
        org_uuid -> Text,
        revision -> BigInt,
        hash -> Text,
        body -> Text,
        envelope -> Nullable<Text>,
        state -> Text,
        created_by -> Text,
        created_at -> BigInt,
        activated_at -> Nullable<BigInt>,
    }
}

table! {
    reins_org_member_policies (org_uuid, user_uuid) {
        org_uuid -> Text,
        user_uuid -> Text,
        body -> Text,
        hash -> Text,
        device_uuid -> Text,
        version -> BigInt,
        at -> BigInt,
        sig -> Text,
        updated_at -> BigInt,
    }
}

table! {
    reins_org_requests (uuid) {
        uuid -> Text,
        org_uuid -> Text,
        user_uuid -> Text,
        connection_uuid -> Text,
        tool -> Text,
        digest -> Text,
        kind -> Text,
        state -> Text,
        outcome -> Text,
        snapshot -> Text,
        member_policy -> Nullable<Text>,
        trace -> Text,
        obligations -> Text,
        decision -> Nullable<Text>,
        created_at -> BigInt,
        expires_at -> BigInt,
        decided_at -> Nullable<BigInt>,
        valid_until -> Nullable<BigInt>,
        claimed_by -> Nullable<Text>,
        claimed_at -> Nullable<BigInt>,
        finished_at -> Nullable<BigInt>,
        receipt -> Nullable<Text>,
    }
}

table! {
    reins_org_bodies (request_uuid, part) {
        request_uuid -> Text,
        part -> Text,
        org_uuid -> Text,
        key_id -> Text,
        sealed -> Text,
        created_at -> BigInt,
        purge_at -> Nullable<BigInt>,
    }
}

table! {
    reins_org_approvals (subject_uuid, user_uuid, decision) {
        subject_uuid -> Text,
        user_uuid -> Text,
        decision -> Text,
        org_uuid -> Text,
        device_uuid -> Text,
        at -> BigInt,
        sig -> Text,
        obligation -> Nullable<Text>,
    }
}

table! {
    reins_org_executors (org_uuid, executor_id) {
        org_uuid -> Text,
        executor_id -> Text,
        mode -> Text,
        public_key -> Text,
        created_at -> BigInt,
        last_seen -> Nullable<BigInt>,
        revoked_at -> Nullable<BigInt>,
    }
}

table! {
    reins_org_leases (uuid) {
        uuid -> Text,
        org_uuid -> Text,
        user_uuid -> Text,
        connection_uuid -> Text,
        request_uuid -> Text,
        repo -> Text,
        access -> Text,
        box_key -> Text,
        state -> Text,
        sealed -> Nullable<Text>,
        created_at -> BigInt,
        expires_at -> BigInt,
        issued_at -> Nullable<BigInt>,
        revoked_at -> Nullable<BigInt>,
        revoke_done_at -> Nullable<BigInt>,
    }
}

table! {
    reins_org_checkpoints (org_uuid, seq) {
        org_uuid -> Text,
        seq -> BigInt,
        hash -> Text,
        at -> BigInt,
        envelope -> Text,
    }
}

table! {
    reins_circle_invites (id) {
        id -> Text,
        token_hash -> Text,
        owner_uuid -> Text,
        kind -> Text,
        state -> Text,
        owner_pub -> Text,
        owner_epoch -> BigInt,
        commitment -> Text,
        helper_uuid -> Nullable<Text>,
        data -> Text,
        link_id -> Nullable<Text>,
        created_at -> BigInt,
        expires_at -> BigInt,
        updated_at -> BigInt,
    }
}

table! {
    reins_circle_links (id) {
        id -> Text,
        owner_uuid -> Text,
        helper_uuid -> Text,
        kind -> Text,
        state -> Text,
        owner_copy -> Text,
        helper_copy -> Text,
        last_change -> Nullable<Text>,
        created_at -> BigInt,
        updated_at -> BigInt,
    }
}

table! {
    reins_circle_packets (id) {
        id -> Text,
        link_id -> Text,
        owner_uuid -> Text,
        helper_uuid -> Text,
        kind -> Text,
        state -> Text,
        sealed -> Text,
        sealed_answer -> Nullable<Text>,
        deadline -> BigInt,
        created_at -> BigInt,
        answered_at -> Nullable<BigInt>,
        delivered -> Bool,
        owner_seen -> Bool,
        updated_at -> BigInt,
    }
}

table! {
    reins_families (org_uuid) {
        org_uuid -> Text,
        notice_days -> Integer,
        created_at -> BigInt,
    }
}

table! {
    reins_family_invites (id) {
        id -> Text,
        token_hash -> Text,
        org_uuid -> Text,
        email -> Text,
        minor -> Bool,
        guardian -> Bool,
        approved -> Bool,
        created_by -> Text,
        created_at -> BigInt,
        expires_at -> BigInt,
        used_at -> Nullable<BigInt>,
        used_by -> Nullable<Text>,
    }
}

table! {
    reins_family_members (org_uuid, user_uuid) {
        org_uuid -> Text,
        user_uuid -> Text,
        minor -> Bool,
        minor_proposed_at -> Nullable<BigInt>,
        minor_proposed_by -> Nullable<Text>,
    }
}

table! {
    reins_family_leaves (org_uuid, user_uuid) {
        org_uuid -> Text,
        user_uuid -> Text,
        requested_at -> BigInt,
        effective_at -> Nullable<BigInt>,
        consented_by -> Nullable<Text>,
    }
}

table! {
    reins_family_connections (org_uuid, user_uuid, provider) {
        org_uuid -> Text,
        user_uuid -> Text,
        provider -> Text,
        account -> Nullable<Text>,
        state -> Text,
        connected_at -> Nullable<BigInt>,
        updated_at -> BigInt,
    }
}

table! {
    reins_family_connect_jobs (id) {
        id -> Text,
        org_uuid -> Text,
        user_uuid -> Text,
        provider -> Text,
        phase -> Text,
        oauth_state -> Nullable<Text>,
        code_key_id -> Nullable<Text>,
        sealed_code -> Nullable<Text>,
        auth_url -> Nullable<Text>,
        state -> Text,
        claimed_by -> Nullable<Text>,
        requested_by -> Text,
        created_at -> BigInt,
        expires_at -> BigInt,
        done_at -> Nullable<BigInt>,
    }
}

joinable!(archives -> users (user_uuid));
joinable!(archives -> ciphers (cipher_uuid));
joinable!(attachments -> ciphers (cipher_uuid));
joinable!(ciphers -> organizations (organization_uuid));
joinable!(ciphers -> users (user_uuid));
joinable!(ciphers_collections -> ciphers (cipher_uuid));
joinable!(ciphers_collections -> collections (collection_uuid));
joinable!(collections -> organizations (org_uuid));
joinable!(devices -> users (user_uuid));
joinable!(folders -> users (user_uuid));
joinable!(folders_ciphers -> ciphers (cipher_uuid));
joinable!(folders_ciphers -> folders (folder_uuid));
joinable!(org_policies -> organizations (org_uuid));
joinable!(sends -> organizations (organization_uuid));
joinable!(sends -> users (user_uuid));
joinable!(twofactor -> users (user_uuid));
joinable!(users_collections -> collections (collection_uuid));
joinable!(users_collections -> users (user_uuid));
joinable!(users_organizations -> organizations (org_uuid));
joinable!(users_organizations -> users (user_uuid));
joinable!(users_organizations -> ciphers (org_uuid));
joinable!(organization_api_key -> organizations (org_uuid));
joinable!(emergency_access -> users (grantor_uuid));
joinable!(groups -> organizations (organizations_uuid));
joinable!(groups_users -> users_organizations (users_organizations_uuid));
joinable!(groups_users -> groups (groups_uuid));
joinable!(collections_groups -> collections (collections_uuid));
joinable!(collections_groups -> groups (groups_uuid));
joinable!(event -> users_organizations (uuid));
joinable!(auth_requests -> users (user_uuid));
joinable!(sso_users -> users (user_uuid));

allow_tables_to_appear_in_same_query!(reins_connections, reins_refresh_tokens);

allow_tables_to_appear_in_same_query!(
    archives,
    attachments,
    ciphers,
    ciphers_collections,
    collections,
    devices,
    folders,
    folders_ciphers,
    invitations,
    org_policies,
    organizations,
    sends,
    sso_users,
    twofactor,
    users,
    users_collections,
    users_organizations,
    organization_api_key,
    emergency_access,
    groups,
    groups_users,
    collections_groups,
    event,
    auth_requests,
);
