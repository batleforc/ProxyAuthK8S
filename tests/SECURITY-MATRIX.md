# Matrice STRIDE → tests négatifs

Une ligne par menace `T<n>` du §7.1 des RFC 0010 à 0014 (RFC 0015 §6.4,
décision 10) : le test qui tente le contournement et prouve qu'il est refusé.
`task it:stride` (et donc `task lint`) échoue si une menace manque, si une
ligne `test` nomme un test absent, si une ligne `hors suite` n'a ni renvoi ni
raison, ou si une ligne `à écrire` appartient à une RFC hors de `PENDING`
(`.it/stride.py`).

Statuts :

- `test` : prouvé par les tests nommés, `chemin/depuis/la/racine.rs::fonction`.
- `hors suite` : prouvé ailleurs (job CI, admission) ou risque accepté dans la
  RFC, avec le renvoi ou la raison.
- `à écrire` : accepté seulement pour une RFC de `PENDING` ; la preuve décrit
  le test à écrire.

Une menace nouvelle d'une RFC suivante entre ici avec sa RFC dans `RFCS`.

| RFC  | T   | Statut | Preuve |
| ---- | --- | ------ | ------ |
| 0010 | T1 | à écrire | un jeton du cluster (autre émetteur) présenté sur un cluster en échange répond 401 sans appel à l'IdP du cluster (`a_cluster_token_is_refused_on_an_exchange_cluster`) |
| 0010 | T2 | à écrire | un non-membre de `proxy_group` est refusé et l'IdP du cluster (wiremock) reçoit 0 appel d'échange (`a_non_member_never_reaches_the_idp`) |
| 0010 | T3 | à écrire | le jeton échangé n'apparaît ni dans la réponse, ni dans les en-têtes renvoyés, ni dans un `Debug` (`the_exchanged_token_never_reaches_the_client`) |
| 0010 | T4 | à écrire | deux utilisateurs sur le même cluster et la même audience reçoivent chacun leur propre jeton échangé du cache (`two_users_never_share_an_exchanged_token`) |
| 0010 | T5 | à écrire | un client qui envoie plusieurs `Authorization` voit l'apiserver ne recevoir que le jeton échangé (`client_authorization_headers_are_all_replaced`) |
| 0010 | T6 | à écrire | un appelant au-delà de la limite reçoit 429 sans aucun appel d'échange à l'IdP (`a_rate_limited_caller_never_reaches_the_idp`) |
| 0010 | T7 | à écrire | un échange écrit une seule ligne d'audit `token_exchanged` sans le jeton (`an_exchange_writes_one_audit_event`) |
| 0010 | T8 | à écrire | un issuer d'échange différent de celui du proxy sans `connector_id` produit l'avertissement (`a_foreign_issuer_without_connector_warns`) |
| 0010 | T9 | à écrire | un jeton échangé en cache n'est plus servi après `max_cache_seconds`, et la CRD refuse plus de 3600 (`the_cache_never_outlives_max_cache_seconds`) |
| 0010 | T10 | à écrire | un échange en échec ne relaie jamais le jeton du proxy à l'apiserver (`an_exchange_failure_never_relays_the_proxy_token`) |
| 0010 | T11 | à écrire | un jeton du proxy émis pour une autre audience est refusé à la porte 4 et n'est jamais envoyé à l'échange (`only_a_validated_token_is_sent_to_the_exchange`) |
| 0010 | T12 | à écrire | un `invalid_grant` sur `/api/v1/auth/refresh` compte vers un bannissement, un IdP injoignable non (`a_refused_refresh_counts_toward_a_ban`) |
| 0011 | T1 | hors suite | `cargo deny check sources` (allow-git limité au dépôt du kit) dans `task audit`, commit figé par `Cargo.lock`, `kit:sync` relu en PR |
| 0011 | T2 | test | `libs/server/api/tests/envtest_reconcile.rs::an_unresolved_oidc_config_never_shows_the_client_secret` |
| 0011 | T3 | test | `tests/integration/tests/rfc0011.rs::t3_the_events_grant_is_create_and_patch_only` |
| 0011 | T4 | test | `tests/integration/tests/rfc0011.rs::p6_a_healthy_reconcile_writes_nothing`, `libs/server/controller/src/proxy_kube_api/mod.rs::error_policy_on_the_leader_backs_off_from_thirty_seconds` |
| 0011 | T5 | test | `tests/integration/tests/rfc0011.rs::p7_only_the_leader_reports`, `libs/server/api/tests/envtest_reconcile.rs::a_follower_does_not_touch_the_resource` |
| 0011 | T6 | test | `libs/server/controller/tests/reconcile.rs::cleanup_fails_closed_when_redis_is_unreachable` |
| 0011 | T7 | test | `libs/server/api/tests/envtest_leader_election.rs::a_frozen_leader_steps_down_before_its_lease_can_be_taken`, `tests/integration/tests/rfc0011_leader.rs::p12_a_foreign_holder_makes_the_leader_step_down` |
| 0011 | T8 | hors suite | risque résiduel accepté (décision 27) : fenêtre du seul rolling upgrade de la phase 4, la relève est montrée par le banc p13 |
| 0012 | T1 | test | `libs/server/api/tests/agent_tunnel.rs::a_cert_from_another_ca_is_refused`, `tests/integration/tests/rfc0012.rs::p12_no_agent_certificate_no_tunnel_and_no_user_route` |
| 0012 | T2 | test | `libs/server/api/tests/agent_tunnel.rs::an_agent_cannot_serve_another_proxykubeapi`, `tests/integration/tests/rfc0012.rs::p7_an_agent_cannot_serve_another_cluster` |
| 0012 | T3 | test | `libs/server/api/tests/agent_tunnel.rs::the_agent_only_sees_tls_records`, `libs/server/crd/src/service/agent.rs::system_roots_are_refused_for_an_agent` |
| 0012 | T4 | test | `libs/server/api/tests/agent_tunnel.rs::spec_cert_is_required_and_system_roots_refused`, `libs/server/api/tests/envtest_agent_crd.rs::system_roots_are_refused_for_an_agent` |
| 0012 | T5 | test | `libs/server/api/tests/agent_tunnel.rs::a_stream_open_carries_no_destination` |
| 0012 | T6 | test | `libs/server/api/tests/agent_tunnel.rs::a_one_shot_token_is_accepted_once` |
| 0012 | T7 | test | `libs/server/api/tests/agent_tunnel.rs::a_token_for_another_subject_or_audience_is_refused`, `tests/integration/tests/rfc0012.rs::p6_another_service_accounts_token_is_refused` |
| 0012 | T8 | test | `libs/server/api/tests/agent_tunnel.rs::an_epoch_bump_closes_tunnels_and_refuses_renewal`, `libs/server/api/tests/envtest_agent_crd.rs::the_epoch_never_decreases`, `tests/integration/tests/rfc0012.rs::p5_an_epoch_bump_revokes_and_the_agent_comes_back` |
| 0012 | T9 | test | `libs/server/api/tests/agent_tunnel.rs::enrollment_never_returns_the_private_key`, `libs/server/api/tests/agent_tunnel.rs::a_one_shot_token_is_accepted_once` |
| 0012 | T10 | test | `libs/server/api/tests/agent_tunnel.rs::the_enroll_route_is_rate_limited_before_any_write` |
| 0012 | T11 | test | `libs/server/api/tests/agent_tunnel.rs::stream_and_tunnel_limits_hold` |
| 0012 | T12 | test | `libs/server/api/tests/agent_tunnel.rs::the_ca_key_never_reaches_a_log` |
| 0012 | T13 | test | `libs/server/api/tests/agent_tunnel.rs::jwks_url_refuses_http_and_redirects`, `libs/server/jwt_validator/src/jwks.rs::a_jwks_past_64_kib_is_refused_before_it_is_all_read`, `libs/server/api/tests/envtest_agent_crd.rs::a_plain_http_jwks_url_is_refused` |
| 0012 | T14 | test | `libs/server/api/tests/agent_tunnel.rs::enrollment_and_tunnels_write_audit_events`, `libs/server/api/tests/agent_tunnel.rs::a_relayed_request_is_audited_as_upstream_via_agent` |
| 0012 | T15 | test | `libs/server/api/tests/agent_tunnel.rs::user_routes_are_not_served_on_the_agent_port`, `tests/integration/tests/rfc0012.rs::p12_no_agent_certificate_no_tunnel_and_no_user_route` |
| 0012 | T16 | test | `tests/integration/tests/rfc0012.rs::p8_one_one_shot_token_serves_two_replicas_and_rotates` |
| 0012 | T17 | test | `libs/server/api/tests/agent_tunnel.rs::a_jwks_from_change_is_picked_up_and_an_empty_one_refuses`, `tests/integration/tests/rfc0012.rs::p13_a_jwks_from_without_the_key_refuses_then_recovers` |
| 0012 | T18 | test | `libs/server/api/src/agent/connect.rs::a_connect_without_the_proxy_token_is_refused`, `tests/integration/tests/rfc0012.rs::p15_a_connect_without_the_proxy_token_is_refused` |
| 0012 | T19 | test | `libs/server/common/src/agent_gateway.rs::a_route_names_a_gateway_pod_never_an_address`, `libs/server/api/tests/agent_tunnel.rs::a_presence_names_a_gateway_pod_never_an_address` |
| 0013 | T1 | test | `libs/server/api/tests/agent_tunnel.rs::an_invalid_announce_is_served_but_never_recorded_as_sent`, `libs/agent/proto/src/lib.rs::only_a_short_semver_is_a_version`, `tests/integration/tests/rfc0013.rs::p6_a_forged_version_is_recorded_invalid` |
| 0013 | T2 | hors suite | risque accepté (décision 3) : la version n'autorise rien, seul le protocole est négocié |
| 0013 | T3 | test | `libs/agent/proto/src/lib.rs::negotiation_keeps_the_highest_protocol_of_both_ranges`, `libs/server/api/tests/agent_tunnel.rs::a_cert_from_another_ca_is_refused` |
| 0013 | T4 | test | `libs/server/api/tests/agent_tunnel.rs::a_refusal_loop_writes_one_bounded_key_and_an_unverified_handshake_none` |
| 0013 | T5 | test | `libs/server/api/tests/agent_tunnel.rs::a_protocol_range_without_overlap_is_refused_before_register` |
| 0013 | T6 | hors suite | risque accepté (mainteneur) : lire la CR exige déjà le RBAC qui montre `spec`, aucune exposition nouvelle |
| 0013 | T7 | test | `tests/integration/tests/rfc0013.rs::p5_a_protocol_without_overlap_is_refused_then_recovers` |
| 0013 | T8 | test | `apps/agent/src/lib.rs::only_an_it_announce_build_reads_the_announce_overrides`, `tests/integration/tests/rfc0013.rs::p7_the_publishable_image_has_no_announce_override` |
| 0014 | T1 | test | `libs/server/api/src/cluster/redirect/forwarded.rs::upstream_auth_headers_are_stripped`, `libs/server/api/src/cluster/redirect/kube_redirect/upgrade.rs::upstream_auth_headers_from_the_client_are_dropped`, `tests/integration/tests/rfc0014.rs::p4_client_identity_headers_never_reach_the_apiserver` |
| 0014 | T2 | test | `libs/server/api/src/cluster/redirect/upstream_identity.rs::a_system_group_from_the_idp_is_prefixed_or_refused`, `tests/integration/tests/rfc0014.rs::p5_a_system_masters_group_is_forwarded_prefixed_and_grants_nothing` |
| 0014 | T3 | test | `libs/server/crd/src/upstream_auth/mod.rs::a_prefix_must_be_safe_to_forward`, `libs/server/crd/src/lib.rs::trusted_modes_refuse_what_would_make_the_asserted_identity_unsafe` |
| 0014 | T4 | test | `libs/server/api/src/cluster/redirect/upstream_identity.rs::a_value_a_header_cannot_carry_is_refused_not_cleaned`, `libs/server/api/src/cluster/redirect/upstream_identity.rs::passthrough_never_forwards_a_line_break` |
| 0014 | T5 | test | `libs/server/api/src/cluster/redirect/kube_redirect/upstream.rs::front_proxy_strips_every_authorization_value_and_asserts_the_identity`, `libs/server/api/src/cluster/redirect/kube_redirect/upstream.rs::issuer_replaces_the_token_with_the_proxy_jwt`, `libs/server/api/src/cluster/redirect/kube_redirect/upgrade/connect.rs::bearer_subprotocol_entries_are_stripped_in_every_mode` |
| 0014 | T6 | test | `tests/integration/tests/rfc0014.rs::p8_identity_headers_without_the_certificate_are_refused`, `libs/server/crd/src/upstream_auth/front_proxy.rs::a_secret_without_the_label_or_for_another_target_is_never_used` |
| 0014 | T7 | test | `libs/server/controller/src/proxy_kube_api/front_proxy.rs::the_subject_and_validity_are_read_and_a_system_subject_is_flagged`, `tests/integration/tests/rfc0014.rs::p9_a_certificate_of_an_untrusted_ca_closes_the_cluster`, `libs/server/api/src/upstream_probe.rs::another_identity_or_a_lost_header_is_not_honoured` |
| 0014 | T8 | test | `libs/server/crd/src/upstream_auth/issuer_key.rs::a_revocation_removes_the_active_key_at_once`, `tests/integration/tests/rfc0014.rs::p13_a_rotation_loses_no_request_and_a_revocation_replaces_the_key` |
| 0014 | T9 | test | `libs/server/api/src/issuer.rs::a_signed_jwt_verifies_against_the_published_key` |
| 0014 | T10 | test | `libs/server/api/src/cluster/redirect/kube_redirect/upstream.rs::issuer_replaces_the_token_with_the_proxy_jwt`, `libs/server/api/src/issuer.rs::an_issued_jwt_never_outlives_the_source_token` |
| 0014 | T11 | test | `libs/server/api/src/issuer.rs::a_group_change_never_reuses_a_cached_jwt` |
| 0014 | T12 | test | `libs/server/api/src/issuer.rs::two_identities_never_share_a_cached_jwt` |
| 0014 | T13 | test | `libs/server/crd/src/lib.rs::trusted_modes_refuse_what_would_make_the_asserted_identity_unsafe`, `tests/integration/tests/rfc0014.rs::p5_a_missing_username_claim_is_refused_by_the_proxy` |
| 0014 | T14 | hors suite | l'apiserver épingle la CA (`certificateAuthority`) ; qui contrôle l'Ingress du proxy contrôle déjà le proxy, risque accepté |
| 0014 | T15 | test | `libs/server/api/tests/issuer_routes.rs::the_issuer_routes_are_never_throttled_and_answer_404_off_issuer`, `libs/server/api/src/issuer.rs::stale_documents_answer_only_as_the_outage_fallback` |
| 0014 | T16 | test | `tests/integration/tests/rfc0014.rs::p3_the_apiserver_sees_the_prefixed_identity` |
| 0014 | T17 | test | `libs/server/api/src/cluster/redirect/kube_redirect/tls.rs::the_ca_only_client_of_a_front_proxy_cluster_presents_no_certificate` |
| 0014 | T18 | test | `libs/server/crd/src/upstream_auth/issuer_key.rs::a_secret_without_the_expected_labels_is_never_adopted`, `libs/server/crd/src/upstream_auth/front_proxy.rs::a_secret_without_the_label_or_for_another_target_is_never_used` |
| 0014 | T19 | test | `libs/server/api/src/cluster/redirect/kube_redirect.rs::a_trusted_mode_outside_allowed_modes_is_refused_with_who_can_act` |
| 0014 | T20 | test | `libs/server/controller/src/proxy_kube_api/reconcile.rs::a_trusted_cluster_is_invisible_under_the_legacy_prefix` |
| 0014 | T21 | test | `libs/server/api/src/cluster/redirect/kube_redirect/upstream.rs::front_proxy_strips_every_authorization_value_and_asserts_the_identity`, `libs/server/api/src/cluster/redirect/kube_redirect/upstream.rs::issuer_replaces_the_token_with_the_proxy_jwt` |
| 0014 | T22 | test | `libs/server/api/src/cluster/redirect/upstream_identity.rs::a_value_a_header_cannot_carry_is_refused_not_cleaned`, `libs/server/crd/src/lib.rs::an_extra_key_that_cannot_be_forwarded_is_refused_in_a_trusted_mode` |
| 0014 | T23 | test | `libs/server/jwt_validator/src/validator.rs::a_trusted_mode_refuses_an_unverified_email_as_username`, `libs/server/api/tests/oidc_user.rs::an_unverified_email_is_refused_as_username_in_a_trusted_mode` |
| 0014 | T24 | test | `libs/server/api/tests/oidc_user.rs::a_mapping_from_one_cluster_never_serves_another`, `tests/integration/tests/rfc0014.rs::p15_a_mapping_from_one_cluster_never_serves_another` |
| 0014 | T25 | test | `libs/server/crd/src/lib.rs::trusted_modes_refuse_what_would_make_the_asserted_identity_unsafe`, `tests/integration/tests/rfc0014.rs::p11_admission_refuses_an_issuer_without_its_key_or_with_a_client_cert` |
| 0014 | T26 | test | `libs/server/controller/src/lib.rs::the_generated_crd_knows_upstream_auth_and_an_older_one_does_not` |
| 0014 | T27 | test | `tests/integration/tests/rfc0014.rs::p11_admission_refuses_a_client_cert_in_front_proxy`, `libs/server/crd/src/upstream_auth/front_proxy.rs::the_secret_name_is_unambiguous` |
| 0014 | T28 | test | `libs/server/api/tests/audience_validation_off.rs::a_trusted_mode_without_audience_validation_is_refused` ; le `Ready=False` (`AudienceValidationRequired`) du contrôleur reste sans test |
| 0014 | T29 | test | `tests/integration/tests/rfc0014.rs::t29_a_certificate_bound_to_another_target_closes_the_cluster`, `libs/server/crd/src/upstream_auth/front_proxy.rs::a_secret_without_the_label_or_for_another_target_is_never_used` |
