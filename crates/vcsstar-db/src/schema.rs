// @generated automatically by Diesel CLI.

diesel::table! {
    act_as_grants (grant_id) {
        grant_id -> Text,
        identity_id -> Uuid,
        grantee -> Text,
        scopes -> Array<Nullable<Text>>,
        expires_at -> Timestamptz,
        revoked_at -> Nullable<Timestamptz>,
        last_presentation_nonce_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    act_as_nonces (nonce) {
        nonce -> Text,
        seen_at -> Timestamptz,
    }
}

diesel::table! {
    api_tokens (id) {
        id -> Uuid,
        identity_id -> Uuid,
        token_hash -> Bytea,
        name -> Text,
        scopes -> Array<Nullable<Text>>,
        client_label -> Nullable<Text>,
        created_at -> Timestamptz,
        expires_at -> Nullable<Timestamptz>,
        revoked_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    app_key_cache (identity_id, application_id, instance_id) {
        identity_id -> Uuid,
        application_id -> Text,
        instance_id -> Text,
        raw_cbor -> Bytea,
        fetched_at -> Timestamptz,
    }
}

diesel::table! {
    audit_log (id) {
        id -> Uuid,
        actor -> Nullable<Uuid>,
        acting_via -> Nullable<Text>,
        action -> Text,
        resource -> Text,
        detail_cbor -> Nullable<Bytea>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    branch_protections (id) {
        id -> Uuid,
        repo_id -> Uuid,
        ref_pattern -> Text,
        required_approvals -> Int4,
        dismiss_stale -> Bool,
        required_signature_level -> Text,
        required_checks -> Array<Nullable<Text>>,
        allowed_merge_kinds -> Array<Nullable<Text>>,
        allow_force_push -> Bool,
        allow_delete -> Bool,
    }
}

diesel::table! {
    credentials (id) {
        id -> Uuid,
        org_id -> Uuid,
        repo_id -> Nullable<Uuid>,
        provider -> Text,
        purpose -> Text,
        auth_method -> Text,
        host -> Text,
        secret_ref -> Text,
        secret_ciphertext -> Nullable<Bytea>,
        enabled -> Bool,
        created_by -> Nullable<Uuid>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    device_logins (id) {
        id -> Uuid,
        code_hash -> Bytea,
        user_code_hash -> Bytea,
        client_label -> Text,
        scopes -> Array<Nullable<Text>>,
        state -> Text,
        identity_id -> Nullable<Uuid>,
        api_token_id -> Nullable<Uuid>,
        created_at -> Timestamptz,
        expires_at -> Timestamptz,
    }
}

diesel::table! {
    events (lane, seq) {
        lane -> Text,
        seq -> Int8,
        repo_id -> Nullable<Uuid>,
        org_id -> Nullable<Uuid>,
        kind -> Text,
        actor -> Nullable<Uuid>,
        subject -> Nullable<Text>,
        payload_cbor -> Bytea,
        created_at -> Timestamptz,
        origin -> Nullable<Text>,
        dispatched_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    github_identities (identity_id) {
        identity_id -> Uuid,
        github_id -> Int8,
        login -> Text,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    github_lanes (lane) {
        lane -> Text,
        consecutive_failures -> Int4,
        opens -> Int4,
        open_until -> Nullable<Timestamptz>,
        probing_since -> Nullable<Timestamptz>,
        last_error -> Nullable<Text>,
        last_failure_at -> Nullable<Timestamptz>,
        last_success_at -> Nullable<Timestamptz>,
        recovered_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    github_link_attempts (state_hash) {
        state_hash -> Bytea,
        identity_id -> Uuid,
        return_to -> Nullable<Text>,
        created_at -> Timestamptz,
        expires_at -> Timestamptz,
    }
}

diesel::table! {
    github_links (identity_id) {
        identity_id -> Uuid,
        github_id -> Int8,
        login -> Text,
        kind -> Text,
        access_ciphertext -> Bytea,
        access_expires_at -> Nullable<Timestamptz>,
        refresh_ciphertext -> Nullable<Bytea>,
        refresh_expires_at -> Nullable<Timestamptz>,
        broken_at -> Nullable<Timestamptz>,
        broken_reason -> Nullable<Text>,
        linked_at -> Timestamptz,
        updated_at -> Timestamptz,
        last_used_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    github_rate_limits (token_label) {
        token_label -> Text,
        remaining -> Int8,
        limit -> Int8,
        reset_at -> Timestamptz,
        hold_until -> Nullable<Timestamptz>,
        seen_at -> Timestamptz,
    }
}

diesel::table! {
    hook_deliveries (id) {
        id -> Uuid,
        endpoint_id -> Uuid,
        event_lane -> Text,
        event_seq -> Int8,
        event_kind -> Text,
        state -> Text,
        attempts -> Int4,
        next_attempt_at -> Timestamptz,
        last_status -> Nullable<Int4>,
        last_error -> Nullable<Text>,
        delivered_at -> Nullable<Timestamptz>,
        payload_cbor -> Bytea,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    hook_endpoints (id) {
        id -> Uuid,
        org_id -> Uuid,
        repo_id -> Nullable<Uuid>,
        kind -> Text,
        url -> Nullable<Text>,
        corndogs_queue -> Nullable<Text>,
        secret -> Nullable<Bytea>,
        event_filter -> Array<Nullable<Text>>,
        content_type -> Text,
        active -> Bool,
        unordered -> Bool,
        created_by -> Nullable<Uuid>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    identities (id) {
        id -> Uuid,
        lk_user_id -> Uuid,
        lk_domain -> Text,
        handle -> Text,
        display_name -> Nullable<Text>,
        email -> Nullable<Text>,
        email_verified -> Bool,
        first_seen -> Timestamptz,
        last_login -> Nullable<Timestamptz>,
        disabled_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    identity_claims (identity_id, claim_type) {
        identity_id -> Uuid,
        claim_type -> Text,
        claim_cbor -> Bytea,
        attested_at -> Nullable<Timestamptz>,
        expires_at -> Nullable<Timestamptz>,
        verified_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    instance_keys (key_id) {
        key_id -> Text,
        instance_id -> Text,
        role -> Text,
        public_key -> Bytea,
        private_sealed -> Bytea,
        attested_at -> Nullable<Timestamptz>,
        attestation_expires_at -> Nullable<Timestamptz>,
        created_at -> Timestamptz,
        revoked_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    lane_counters (lane) {
        lane -> Text,
        next_seq -> Int8,
    }
}

diesel::table! {
    mirror_actions (id) {
        id -> Uuid,
        repo_id -> Uuid,
        event_lane -> Text,
        event_seq -> Int8,
        action_kind -> Text,
        object_key -> Text,
        state -> Text,
        blocked_reason -> Nullable<Text>,
        blocked_at -> Nullable<Timestamptz>,
        credential_purpose -> Text,
        payload_cbor -> Bytea,
        attempts -> Int4,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    mirror_attempts (id) {
        id -> Uuid,
        action_id -> Uuid,
        started_at -> Timestamptz,
        finished_at -> Nullable<Timestamptz>,
        ok -> Bool,
        http_status -> Nullable<Int4>,
        error -> Nullable<Text>,
    }
}

diesel::table! {
    mirror_refs (repo_id, ref_) {
        repo_id -> Uuid,
        #[sql_name = "ref"]
        ref_ -> Text,
        mirrored_sha -> Text,
        last_pushed_at -> Nullable<Timestamptz>,
        last_verified_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    mirror_state (repo_id) {
        repo_id -> Uuid,
        github_repo -> Text,
        mode -> Text,
        paused -> Bool,
        cursor_seq -> Int8,
        last_success_at -> Nullable<Timestamptz>,
        last_error -> Nullable<Text>,
        last_full_reconcile_at -> Nullable<Timestamptz>,
        created_at -> Timestamptz,
        last_pull_sync_at -> Nullable<Timestamptz>,
        last_pull_pass_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    org_members (org_id, identity_id) {
        org_id -> Uuid,
        identity_id -> Uuid,
        role -> Text,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    orgs (id) {
        id -> Uuid,
        name -> Text,
        display_name -> Nullable<Text>,
        github_owner -> Nullable<Text>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    pending_logins (id) {
        id -> Uuid,
        attempt_hash -> Bytea,
        nonce -> Text,
        identity_input -> Text,
        idp_domain -> Text,
        return_to -> Nullable<Text>,
        device_login_id -> Nullable<Uuid>,
        created_at -> Timestamptz,
        expires_at -> Timestamptz,
        claimed_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    pr_comments (id) {
        id -> Uuid,
        pr_id -> Uuid,
        author -> Uuid,
        body -> Text,
        path -> Nullable<Text>,
        line -> Nullable<Int4>,
        commit_sha -> Nullable<Text>,
        created_at -> Timestamptz,
        edited_at -> Nullable<Timestamptz>,
        github_id -> Nullable<Int8>,
        github_kind -> Nullable<Text>,
        github_updated_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    pr_counters (repo_id) {
        repo_id -> Uuid,
        next_number -> Int4,
    }
}

diesel::table! {
    pr_reviews (id) {
        id -> Uuid,
        pr_id -> Uuid,
        reviewer -> Uuid,
        state -> Text,
        head_sha_at_review -> Text,
        body -> Nullable<Text>,
        created_at -> Timestamptz,
        github_id -> Nullable<Int8>,
    }
}

diesel::table! {
    pull_requests (id) {
        id -> Uuid,
        repo_id -> Uuid,
        number -> Int4,
        title -> Text,
        body -> Nullable<Text>,
        author_identity_id -> Uuid,
        head_repo_id -> Nullable<Uuid>,
        head_remote -> Nullable<Text>,
        head_ref -> Text,
        base_ref -> Text,
        head_sha -> Text,
        base_sha -> Text,
        merge_base_sha -> Nullable<Text>,
        mergeable -> Text,
        state -> Text,
        merge_kind -> Nullable<Text>,
        merged_commit_sha -> Nullable<Text>,
        merged_by -> Nullable<Uuid>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        closed_at -> Nullable<Timestamptz>,
        github_number -> Nullable<Int4>,
        github_updated_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    repo_grants (repo_id, grantee_kind, grantee_id) {
        repo_id -> Uuid,
        grantee_kind -> Text,
        grantee_id -> Uuid,
        role -> Text,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    repo_refs (repo_id, name) {
        repo_id -> Uuid,
        name -> Text,
        target -> Bytea,
        updated_at -> Timestamptz,
        updated_by -> Nullable<Uuid>,
    }
}

diesel::table! {
    repo_segments (repo_id, ordinal) {
        repo_id -> Uuid,
        ordinal -> Int4,
        object_key -> Text,
        size -> Int8,
        content_hash -> Text,
        kind -> Text,
        created_at -> Timestamptz,
        superseded_at -> Nullable<Timestamptz>,
        has_bitmap -> Bool,
    }
}

diesel::table! {
    repos (id) {
        id -> Uuid,
        org_id -> Uuid,
        name -> Text,
        visibility -> Text,
        vcs_type -> Text,
        default_branch -> Text,
        description -> Nullable<Text>,
        fork_of_repo_id -> Nullable<Uuid>,
        fork_of_remote -> Nullable<Text>,
        archived -> Bool,
        mirror_enabled -> Bool,
        refs_version -> Int8,
        created_at -> Timestamptz,
        repacked_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    service_accounts (identity_id) {
        identity_id -> Uuid,
        org_id -> Uuid,
        name -> Text,
        description -> Nullable<Text>,
        created_by -> Nullable<Uuid>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    sessions (id) {
        id -> Uuid,
        identity_id -> Uuid,
        token_hash -> Bytea,
        created_at -> Timestamptz,
        expires_at -> Timestamptz,
        last_used_at -> Nullable<Timestamptz>,
        user_agent -> Nullable<Text>,
    }
}

diesel::table! {
    ssh_keys (id) {
        id -> Uuid,
        identity_id -> Uuid,
        usage -> Text,
        algo -> Text,
        public_key -> Bytea,
        fingerprint -> Text,
        attested -> Bool,
        application_id -> Nullable<Text>,
        instance_id -> Nullable<Text>,
        key_id -> Nullable<Text>,
        attested_until -> Nullable<Timestamptz>,
        created_at -> Timestamptz,
        revoked_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    statuses (id) {
        id -> Uuid,
        repo_id -> Uuid,
        sha -> Text,
        context -> Text,
        state -> Text,
        target_url -> Nullable<Text>,
        description -> Nullable<Text>,
        reporter -> Nullable<Uuid>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    team_members (team_id, identity_id) {
        team_id -> Uuid,
        identity_id -> Uuid,
    }
}

diesel::table! {
    teams (id) {
        id -> Uuid,
        org_id -> Uuid,
        name -> Text,
    }
}

diesel::joinable!(act_as_grants -> identities (identity_id));
diesel::joinable!(api_tokens -> identities (identity_id));
diesel::joinable!(app_key_cache -> identities (identity_id));
diesel::joinable!(branch_protections -> repos (repo_id));
diesel::joinable!(credentials -> identities (created_by));
diesel::joinable!(credentials -> orgs (org_id));
diesel::joinable!(credentials -> repos (repo_id));
diesel::joinable!(device_logins -> api_tokens (api_token_id));
diesel::joinable!(device_logins -> identities (identity_id));
diesel::joinable!(github_identities -> identities (identity_id));
diesel::joinable!(github_link_attempts -> identities (identity_id));
diesel::joinable!(github_links -> identities (identity_id));
diesel::joinable!(hook_deliveries -> hook_endpoints (endpoint_id));
diesel::joinable!(hook_endpoints -> identities (created_by));
diesel::joinable!(hook_endpoints -> orgs (org_id));
diesel::joinable!(hook_endpoints -> repos (repo_id));
diesel::joinable!(identity_claims -> identities (identity_id));
diesel::joinable!(mirror_actions -> repos (repo_id));
diesel::joinable!(mirror_attempts -> mirror_actions (action_id));
diesel::joinable!(mirror_refs -> repos (repo_id));
diesel::joinable!(mirror_state -> repos (repo_id));
diesel::joinable!(org_members -> identities (identity_id));
diesel::joinable!(org_members -> orgs (org_id));
diesel::joinable!(pr_comments -> identities (author));
diesel::joinable!(pr_comments -> pull_requests (pr_id));
diesel::joinable!(pr_counters -> repos (repo_id));
diesel::joinable!(pr_reviews -> identities (reviewer));
diesel::joinable!(pr_reviews -> pull_requests (pr_id));
diesel::joinable!(repo_grants -> repos (repo_id));
diesel::joinable!(repo_refs -> identities (updated_by));
diesel::joinable!(repo_refs -> repos (repo_id));
diesel::joinable!(repo_segments -> repos (repo_id));
diesel::joinable!(repos -> orgs (org_id));
diesel::joinable!(service_accounts -> orgs (org_id));
diesel::joinable!(sessions -> identities (identity_id));
diesel::joinable!(ssh_keys -> identities (identity_id));
diesel::joinable!(statuses -> identities (reporter));
diesel::joinable!(statuses -> repos (repo_id));
diesel::joinable!(team_members -> identities (identity_id));
diesel::joinable!(team_members -> teams (team_id));
diesel::joinable!(teams -> orgs (org_id));

diesel::allow_tables_to_appear_in_same_query!(
    act_as_grants,
    act_as_nonces,
    api_tokens,
    app_key_cache,
    audit_log,
    branch_protections,
    credentials,
    device_logins,
    events,
    github_identities,
    github_lanes,
    github_link_attempts,
    github_links,
    github_rate_limits,
    hook_deliveries,
    hook_endpoints,
    identities,
    identity_claims,
    instance_keys,
    lane_counters,
    mirror_actions,
    mirror_attempts,
    mirror_refs,
    mirror_state,
    org_members,
    orgs,
    pending_logins,
    pr_comments,
    pr_counters,
    pr_reviews,
    pull_requests,
    repo_grants,
    repo_refs,
    repo_segments,
    repos,
    service_accounts,
    sessions,
    ssh_keys,
    statuses,
    team_members,
    teams,
);
