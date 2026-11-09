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
    events (lane, seq, created_at) {
        lane -> Text,
        seq -> Int8,
        repo_id -> Nullable<Uuid>,
        org_id -> Nullable<Uuid>,
        kind -> Text,
        actor -> Nullable<Uuid>,
        subject -> Nullable<Text>,
        payload_cbor -> Bytea,
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
    lane_counters (lane) {
        lane -> Text,
        next_seq -> Int8,
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
diesel::joinable!(device_logins -> api_tokens (api_token_id));
diesel::joinable!(device_logins -> identities (identity_id));
diesel::joinable!(identity_claims -> identities (identity_id));
diesel::joinable!(org_members -> identities (identity_id));
diesel::joinable!(org_members -> orgs (org_id));
diesel::joinable!(repo_grants -> repos (repo_id));
diesel::joinable!(repo_refs -> identities (updated_by));
diesel::joinable!(repo_refs -> repos (repo_id));
diesel::joinable!(repo_segments -> repos (repo_id));
diesel::joinable!(repos -> orgs (org_id));
diesel::joinable!(sessions -> identities (identity_id));
diesel::joinable!(ssh_keys -> identities (identity_id));
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
    device_logins,
    events,
    identities,
    identity_claims,
    lane_counters,
    org_members,
    orgs,
    pending_logins,
    repo_grants,
    repo_refs,
    repo_segments,
    repos,
    sessions,
    ssh_keys,
    team_members,
    teams,
);
