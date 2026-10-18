[0m
> build · gpt-6-luna
[0m→ [0mSkill "code-review" 
> build · gpt-6-luna
我会按代码与文档的实际内容核查，不把“142 项已完成”当作质量结论；重点找有证据、可定位的风险，以及文档承诺和实现行为的偏差。
[0m✱ [0mGlob "**/*" [90m300 matches[0m
[0m✱ [0mGrep "TODO|FIXME|XXX|HACK|142" [90min . · 141 matches[0m
[0m$ [0mgit status --short && git log -5 --oneline
5f432d8 docs: decide prototype ui copy stays chinese
b3f554d fix: redirect docs home visitors by browser language
dfba410 feat: detect browser language on first visit
536277f docs: add english design draft
8ba63d5 docs: add english site navigation
[0m✱ [0mGrep "architecture|ADR|design|spec|README|implementation|implemented|must|shall" [90min . · 300 matches[0m
> build · gpt-6-luna
[0m→ [0mSkill "architecture-review" 
> build · gpt-6-luna
[0m→ [0mRead todo.md [90m[offset=1080, limit=480][0m
[0m→ [0mRead Cargo.toml 
[0m→ [0mRead docs/architecture.md 
[0m→ [0mRead docs/audit-doc-consistency.md 
[0m→ [0mRead README.zh.md [90m[limit=230][0m
[0m$ [0mfind crates -path '*/src/*' -type f | sort
crates/cage-cli/src/init.rs
crates/cage-cli/src/lsp.rs
crates/cage-cli/src/main.rs
crates/cage-cli/src/mcp.rs
crates/cage-cli/src/remote.rs
crates/cage-cli/src/web.rs
crates/cage-core/src/assets/mod.rs
crates/cage-core/src/assets/tests.rs
crates/cage-core/src/diagnostics/mod.rs
crates/cage-core/src/edit/mod.rs
crates/cage-core/src/error/codes.rs
crates/cage-core/src/error/mod.rs
crates/cage-core/src/lib.rs
crates/cage-core/src/manifest/mod.rs
crates/cage-core/src/migrate/diff.rs
crates/cage-core/src/migrate/mod.rs
crates/cage-core/src/normalize/mod.rs
crates/cage-core/src/reference/mod.rs
crates/cage-core/src/registry.rs
crates/cage-core/src/remote.rs
crates/cage-core/src/schema/mod.rs
crates/cage-core/src/snapshot/mod.rs
crates/cage-core/src/validation/expr.rs
crates/cage-core/src/validation/mod.rs
crates/cage-core/src/validation/rules.rs
crates/cage-core/src/value/mod.rs
crates/cage-source-csv/src/lib.rs
crates/cage-source-db/src/draft.rs
crates/cage-source-db/src/lib.rs
crates/cage-source-db/src/mysql.rs
crates/cage-source-db/src/pg.rs
crates/cage-source-db/src/pool.rs
crates/cage-source-excel/src/lib.rs
crates/cage-source-http/src/lib.rs
crates/cage-source-json/src/lib.rs
crates/cage-source-msgpack/src/lib.rs
crates/cage-source-sheets/src/lib.rs
crates/cage-source-yaml/src/lib.rs
crates/cage-target-cpp/src/lib.rs
crates/cage-target-cs/src/lib.rs
crates/cage-target-csv/src/lib.rs
crates/cage-target-flatbuffers/src/lib.rs
crates/cage-target-go/src/lib.rs
crates/cage-target-java/src/lib.rs
crates/cage-target-jsonschema/src/lib.rs
crates/cage-target-json/src/lib.rs
crates/cage-target-lua/src/lib.rs
crates/cage-target-msgpack/src/lib.rs
crates/cage-target-proto/src/lib.rs
crates/cage-target-py/src/lib.rs
crates/cage-target-template/src/lib.rs
crates/cage-target-ts/src/lib.rs
> build · gpt-6-luna
[0m→ [0mRead crates/cage-core/src/registry.rs [90m[limit=260][0m
[0m→ [0mRead crates/cage-cli/src/web.rs [90m[limit=300][0m
[0m→ [0mRead crates/cage-cli/src/lsp.rs [90m[limit=360][0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[limit=340][0m
[0m✱ [0mGrep "TODO|留待|不做|不实现|未实现|暂不|卡点|缺口|尚未" [90min docs · 133 matches[0m
[0m✱ [0mGrep "unsafe|unwrap\(|expect\(|panic!|todo!|unimplemented!" [90min crates · 250 matches[0m
> build · gpt-6-luna
[0m→ [0mRead crates/cage-cli/src/web.rs [90m[offset=291, limit=700][0m
[0m→ [0mRead crates/cage-source-db/src/pg.rs [90m[limit=260][0m
[0m→ [0mRead crates/cage-cli/src/mcp.rs [90m[limit=360][0m
[0m→ [0mRead crates/cage-core/src/snapshot/mod.rs [90m[limit=300][0m
[0m✱ [0mGrep "fn api_artifact|fn api_build|fn api_snapshot|fn api_save|fn api_diff|fn api_registry|path.*canonical|strip_prefix|Origin|Host|Access-Control|Content-Security|same" [90min crates/cage-cli/src/web.rs · 43 matches[0m
[0m→ [0mRead docs/ui.md [90m[limit=290][0m
> build · gpt-6-luna
[0m✱ [0mGrep "fn load_manifest|pub.*load_manifest|manifest.json" [90min crates/cage-cli/src/main.rs · 27 matches[0m
[0m✱ [0mGrep "pub fn import_bundle|fn import_bundle|unpack|EntryType|path_components|verify_snapshot|sign|signature|symlink|entry.path|archive" [90min crates/cage-core/src/registry.rs · 177 matches[0m
[0m→ [0mRead docs/lifecycle.md [90m[limit=240][0m
[0m→ [0mRead docs/cli.md [90m[offset=365, limit=100][0m
[0m→ [0mRead docs/architecture.en.md [90m[offset=195, limit=35][0m
[0m→ [0mRead docs/validation.md [90m[offset=460, limit=185][0m
> build · gpt-6-luna
[0m→ [0mRead crates/cage-core/src/registry.rs [90m[offset=850, limit=180][0m
[0m→ [0mRead docs/requirements.md [90m[limit=130][0m
[0m→ [0mRead crates/cage-core/src/registry.rs [90m[offset=1770, limit=175][0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=4070, limit=60][0m
[0m✱ [0mGrep "Access-Control|Origin|Host|X-Content-Type|X-Frame|Content-Security|same-origin|csrf" [90min crates/cage-cli/src/web.rs · 12 matches[0m
[0m→ [0mRead docs/design.md [90m[offset=3085, limit=80][0m
> build · gpt-6-luna
目前确认两类值得优先核实的点：`cage web` 是无鉴权但带写入/构建端点的本地服务；此外有几处文档仍保留旧口径。接下来我会把这些与具体调用路径对上，并跑仓库门禁，避免把“看起来可疑”当成结论。
[0m→ [0mRead docs/lifecycle.md [90m[offset=129, limit=14][0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=292, limit=40][0m
[0m→ [0mRead crates/cage-cli/src/web.rs [90m[offset=314, limit=84][0m
[0m→ [0mRead crates/cage-cli/src/web.rs [90m[offset=57, limit=70][0m
[0m→ [0mRead crates/cage-core/src/snapshot/mod.rs [90m[offset=98, limit=82][0m
[0m→ [0mRead docs/validation.md [90m[offset=471, limit=18][0m
[0m$ [0mcargo test --workspace --locked
C to stop
cage web: http://127.0.0.1:33075 serving /home/cui/.hermes/cache/scratch/.tmp7VYVF2
cage web: Ctrl+C to stop
cage web: http://127.0.0.1:43529 serving /home/cui/.hermes/cache/scratch/.tmpQEESm8
cage web: Ctrl+C to stop
cage web: http://127.0.0.1:44349 serving /home/cui/.hermes/cache/scratch/.tmpkcBJLv
cage web: Ctrl+C to stop
ERROR E1001 — Missing Required Field
  Source: /home/cui/.hermes/cache/scratch/.tmpkcBJLv/config/item.json | Format: json | Row: 1 | Field: id
  Table: Item
  Row: 0
  Field: id
  Message: Missing required field
  Hint: Add required field 'id' to this row


test web_serves_editor_page_and_assets ... ok
test web_serves_editor_document ... ok
test web_build_refuses_broken_data_with_diagnostics ... ok
test web_serves_empty_schema_when_config_names_none ... ok
test web_save_refuses_directory_schema_path ... ok
test web_save_refuses_registry_schema_path ... ok
test web_serves_status_page_with_redirect_and_assets ... ok
test web_validate_reports_e1701_and_e1004 ... ok
test web_schema_projection_gates_by_profile ... ok
test web_artifacts_serve_ledgered_files_end_to_end ... ok
test web_snapshots_diff_and_registry_end_to_end ... ok
test web_save_writes_canonical_yaml_that_builds_and_reloads ... ok
test web_status_and_build_report_freshness_end_to_end ... ok

test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s

     Running unittests src/lib.rs (target/debug/deps/cage_core-d46ae0a60198af88)

running 275 tests
test diagnostics::tests::test_empty_diagnostics_render_and_json ... ok
test assets::tests::test_verify_rejects_non_inventory_file ... ok
test assets::tests::test_inventory_skips_remote_specs ... ok
test assets::tests::test_verify_rejects_foreign_baseline ... ok
test diagnostics::tests::test_diagnostic_builder ... ok
test diagnostics::tests::test_add_info_extend_iter_and_infos ... ok
test diagnostics::tests::test_iter_mut_escalates_warnings_to_errors ... ok
test diagnostics::tests::test_diagnostic_builder_warning_info_location_related_metadata ... ok
test diagnostics::tests::test_diagnostics_render_and_json ... ok
test assets::tests::test_inventory_is_deterministic ... ok
test assets::tests::test_inventory_classes_paths_and_hashes ... ok
test diagnostics::tests::test_diagnostics_collection ... ok
test assets::tests::test_verify_reports_missing_changed_unlisted ... ok
test diagnostics::tests::test_render_full_fields ... ok
test diagnostics::tests::test_render_without_location_and_display_trait ... ok
test error::codes::tests::default_severity_prefix_families_and_unknown_fallback ... ok
test error::codes::tests::error_title_resolves_every_code_family ... ok
test diagnostics::tests::test_render_color_covers_every_severity_and_hint ... ok
test error::codes::tests::test_default_severity ... ok
test error::codes::tests::test_error_titles ... ok
test edit::tests::e1701_reports_bad_editor_input_with_json_path ... ok
test diagnostics::tests::test_sort_by_location_ordering_with_missing_locations ... ok
test error::codes::tests::severity_display_uses_uppercase_label ... ok
test manifest::tests::target_records_sorted_and_hash_sensitive ... ok
test manifest::tests::test_dependency_ledger_records_references ... ok
test manifest::tests::test_input_hashes_match_generate ... ok
test manifest::tests::test_hash_source_covers_all_value_variants_and_is_stable ... ok
test manifest::tests::test_build_id_rotates_with_semantic_changes ... ok
test manifest::tests::test_manifest_generation ... ok
test manifest::tests::test_deterministic_hashes ... ok
test manifest::tests::test_verify_manifest ... ok
test manifest::tests::test_project_config_roundtrip_full ... ok
test manifest::tests::test_verify_manifest_extra_artifact_is_warning ... ok
test manifest::tests::test_project_config_default_shape_and_roundtrip ... ok
test manifest::tests::test_generate_records_artifact_metadata_and_is_order_insensitive ... ok
test manifest::tests::test_project_config_minimal_json_omits_optionals ... ok
test migrate::diff::tests::has_unsigned_spots_the_family_through_containers ... ok
test migrate::diff::tests::json_to_value_maps_every_json_shape ... ok
test migrate::diff::tests::quote_string_always_reparses_as_a_string_or_refuses ... ok
test migrate::diff::tests::draft_render_is_deterministic_and_todo_only_drafts_render_empty_steps ... ok
test manifest::tests::test_verify_manifest_hash_and_size_mismatch_both_reported ... ok
test manifest::tests::test_verify_manifest_missing_artifact_is_error ... ok
test migrate::diff::tests::enum_member_changes_and_non_widenings_stay_todo ... ok
test migrate::diff::tests::draft_covers_remove_widen_add_default_and_todo_paths ... ok
test migrate::diff::tests::required_flip_backfills_from_default_or_todo ... ok
test migrate::diff::tests::typed_defaults_land_in_the_field_family ... ok
test migrate::tests::apply_is_deterministic_two_runs_identical ... ok
test migrate::diff::tests::typed_json_value_shapes_defaults_into_their_family ... ok
test migrate::tests::apply_over_migrated_document_is_a_zero_change_noop ... ok
test migrate::tests::excel_remap_run_twice_records_no_locations_and_no_rows ... ok
test migrate::diff::tests::render_step_forms_cover_every_variant_and_todo_fallback ... ok
test migrate::diff::tests::render_round_trips_through_parse_spec ... ok
test manifest::tests::test_build_manifest_serialization_roundtrip ... ok
test migrate::tests::apply_runs_all_six_step_semantics ... ok
test migrate::tests::excel_remove_field_and_widen_type_record_affected_rows ... ok
test migrate::tests::is_widening_and_widen_value_cover_every_rank_and_family ... ok
test migrate::tests::excel_rename_records_rows_and_null_defaults_fill_in_place ... ok
test migrate::tests::excel_sourced_steps_record_affected_row_locations ... ok
test migrate::tests::parse_migration_dir_reports_an_unreadable_directory ... ok
test migrate::tests::remap_values_ignores_non_string_values ... ok
test migrate::tests::a_step_must_carry_exactly_one_transform ... ok
test migrate::tests::validate_spec_checks_table_and_field_references_as_e2002 ... ok
test edit::tests::editor_json_round_trip_is_deterministic ... ok
test migrate::diff::tests::added_and_required_flip_todo_arms ... ok
test migrate::tests::rename_collisions_fail_as_e2003_instead_of_collapsing ... ok
test migrate::tests::validate_step_rejects_widen_type_on_an_unknown_field ... ok
test migrate::tests::rename_table_reports_whole_excel_table_and_text_tables_stay_location_free ... ok
test migrate::tests::widen_type_without_a_from_schema_type_is_e2003 ... ok
test migrate::tests::rename_table_without_either_name_is_e2003 ... ok
test migrate::tests::parse_spec_reads_all_six_step_kinds_in_order ... ok
test migrate::tests::widen_type_current_equals_to_skips_direction_table_keeps_domain_check ... ok
test normalize::tests::test_coerce_array_elements ... ok
test migrate::tests::reverify_passes_clean_and_fails_as_e2004 ... ok
test normalize::tests::test_coerce_float_to_integer_boundaries ... ok
test migrate::tests::yaml_to_value_decodes_the_canonical_tagged_form ... ok
test migrate::tests::widen_type_rejects_unsafe_directions_and_unfit_values_as_e2003 ... ok
test migrate::tests::validate_structure_refuses_empty_version_strings ... ok
test migrate::tests::parse_spec_reports_structural_problems_as_e2001 ... ok
test migrate::tests::yaml_to_value_maps_every_scalar_family_and_refuses_tags ... ok
test migrate::tests::parse_migration_dir_orders_segments_by_file_name ... ok
test normalize::tests::test_coerce_int_to_uint ... ok
test normalize::tests::test_coerce_identity_types ... ok
test normalize::tests::test_coerce_int_to_uint_all_widths ... ok
test normalize::tests::test_coerce_numeric_to_bool ... ok
test normalize::tests::test_coerce_map_values_recursively_and_keep_keys ... ok
test normalize::tests::test_coerce_float_to_int ... ok
test normalize::tests::test_coerce_object_fields ... ok
test normalize::tests::test_coerce_rejects_unrelated_pairs ... ok
test normalize::tests::test_coerce_string_bool_case_and_whitespace_insensitive ... ok
test normalize::tests::test_coerce_string_parse_failures ... ok
test normalize::tests::test_coerce_string_to_bool ... ok
test normalize::tests::test_coerce_uint_to_int ... ok
test normalize::tests::test_coerce_string_to_int ... ok
test normalize::tests::test_normalize_array_recursively_in_order ... ok
test edit::tests::editor_json_matches_the_snapshot_schema_shape ... ok
test normalize::tests::test_normalize_object_sorts_keys ... ok
test normalize::tests::test_normalize_passthrough_variants ... ok
test normalize::tests::test_normalize_typed_value_preserves_metadata ... ok
test normalize::tests::test_normalize_float ... ok
test normalize::tests::test_normalize_string ... ok
test normalize::tests::test_normalize_map_field_values_deterministically ... ok
test normalize::tests::test_normalize_document_canonicalizes_and_is_deterministic ... ok
test reference::tests::test_cycle_detection ... ok
test reference::tests::test_cycle_detection_shapes_and_error_message ... ok
test reference::tests::test_dependency_graph ... ok
test reference::tests::test_empty_schema_and_document ... ok
test reference::tests::test_dependency_graph_transitive_dependencies_and_dependents ... ok
test reference::tests::test_get_referencing_rows_reverse_lookup ... ok
test reference::tests::test_build_reverse_deps ... ok
test reference::tests::test_incremental_planner ... ok
test reference::tests::test_reference_resolution ... ok
test reference::tests::test_resolver_skips_unschemaed_tables_and_reports_missing_targets ... ok
test reference::tests::test_self_table_reference_resolution ... ok
test reference::tests::test_topological_sort_diamond_is_acyclic ... ok
test registry::tests::key_env_resolves_seed_and_rejects_bad_material ... ok
test registry::tests::parse_presign_map_accepts_and_enforces_the_shape ... ok
test reference::tests::test_incremental_planner_build_order_and_errors ... ok
test registry::tests::import_bundle_rejects_file_count_mismatch ... ok
test registry::tests::export_bundle_missing_entry_or_bad_name_is_e2101 ... ok
test registry::tests::import_bundle_rejects_malformed_member_structures ... ok
test edit::tests::canonical_yaml_round_trip_is_idempotent ... ok
test registry::tests::parse_spec_accepts_bare_and_pinned_versions ... ok
test registry::tests::parse_version_req_covers_eq_caret_zero_and_tilde_label ... ok
test registry::tests::parse_spec_rejects_path_escape_attempts ... ok
test registry::tests::packages_lists_only_index_backed_dirs ... ok
test registry::tests::export_bundle_is_deterministic_tar_with_entry_files_and_excerpt ... ok
test registry::tests::gc_registry_reports_removals_and_skips_unchanged_packages ... ok
test registry::tests::publish_refuses_unverifiable_snapshot ... ok
test registry::tests::publish_rejects_bad_names_unverifiable_packs_and_blocked_targets ... ok
test registry::tests::import_bundle_refuses_tamper_conflict_and_respects_dry_run ... ok
test registry::tests::publish_same_artifacts_different_build_is_conflict ... ok
test registry::tests::import_bundle_enters_verified_and_is_idempotent ... ok
test registry::tests::publish_same_version_different_bytes_is_conflict ... ok
test registry::tests::read_bundle_signature_rejects_malformed_sidecar ... ok
test registry::tests::read_index_rejects_unreadable_and_corrupt_index ... ok
test registry::tests::publish_resolves_latest_and_is_idempotent ... ok
test registry::tests::resolve_entry_reports_missing_entry_directory ... ok
test registry::tests::remove_entry_rejects_bad_version_and_tolerates_missing_dir ... ok
test registry::tests::resolve_missing_package_or_version_is_e1802 ... ok
test registry::tests::select_version_is_pure_and_drives_the_remote_fetcher ... ok
test registry::tests::resolve_pinned_picks_max_satisfying_and_gates_explicit_pins ... ok
test registry::tests::sidecar_roundtrips_and_missing_sidecar_is_e2107 ... ok
test registry::tests::resolve_tampered_entry_is_e1803 ... ok
test registry::tests::verify_registry_clean_pass_ledger_mismatch_and_named_orphans ... ok
test registry::tests::push_entry_presigned_coverage_gap_and_dry_run_upload_nothing ... ok
test registry::tests::verifying_key_from_env_rejects_empty_and_malformed_values ... ok
test registry::tests::version_order_is_dotted_numeric ... ok
test registry::tests::version_req_expands_caret_and_tilde ... ok
test registry::tests::version_req_rejects_malformed_specs ... ok
test registry::tests::version_req_satisfies_uses_zero_padded_order ... ok
test registry::tests::write_index_reports_create_and_write_failures ... ok
test remote::tests::cache_key_is_deterministic_isolated_and_sized ... ok
test remote::tests::cooldown_is_negotiated_for_rate_limits_backoff_for_transport ... ok
test remote::tests::parse_http_date_matches_known_epochs ... ok
test registry::tests::verify_registry_reports_unreadable_ledger_and_skips_unnameable_dirs ... ok
test remote::tests::parse_retry_after_reads_delta_seconds_and_dates ... ok
test remote::tests::source_cache_dir_derives_from_url_key ... ok
test remote::tests::with_retries_honors_rate_limit_cooldown ... ok
test remote::tests::with_retries_budget_exhausts_and_server_answers_do_not_retry ... ok
test remote::tests::with_retries_recovers_from_transport_flake ... ok
test schema::tests::declared_envs_unions_table_override_keys ... ok
test schema::tests::dependency_graph_sort_handles_revisits_and_cycles ... ok
test schema::tests::field_schema_yaml_parses_map_field ... ok
test schema::tests::field_schema_yaml_parsing_rules_constraints_and_metadata ... ok
test schema::tests::field_type_classification_helpers ... ok
test schema::tests::lint_env_overrides_collects_all_problems ... ok
test schema::tests::map_key_type_wire_format_is_lowercase ... ok
test schema::tests::reference_schema_cardinality_serde_edges ... ok
test registry::tests::verify_gc_and_remove_govern_the_registry_lifecycle ... ok
test schema::tests::resolve_env_patches_fields_and_keeps_base_untouched ... ok
test schema::tests::rust_type_names_for_every_variant ... ok
test schema::tests::schema_default_and_enum_registration ... ok
test schema::tests::schema_lookup_helpers_hit_and_miss ... ok
test schema::tests::schema_validate_reports_every_error_kind ... ok
test schema::tests::test_dependency_graph ... ok
test schema::tests::field_type_yaml_parsing_all_kinds_and_edges ... ok
test schema::tests::test_schema_validation ... ok
test schema::tests::table_schema_yaml_parsing_primary_key_unique_and_order ... ok
test schema::tests::validate_env_overrides_rejects_unknown_fields_and_empty_patches ... ok
test snapshot::tests::snapshot_files_partition_and_ledger_are_deterministic ... ok
test validation::expr::tests::evaluates_numeric_string_and_bool_comparisons ... ok
test validation::expr::tests::missing_fields_pass_and_mixed_types_are_incomparable ... ok
test validation::expr::tests::parser_rejects_malformed_asserts ... ok
test validation::expr::tests::parses_bare_backtick_and_literal_operands ... ok
test snapshot::tests::load_round_trips_manifest_schema_and_artifacts ... ok
test validation::expr::tests::renders_operands_for_diagnostics ... ok
test snapshot::tests::verify_accepts_clean_and_flags_tamper_growth_and_loss ... ok
test validation::rules::tests::builtin_validator_reports_its_name ... ok
test validation::rules::tests::boundary_value_passes ... ok
test validation::rules::tests::custom_validator_registers_and_runs ... ok
test validation::rules::tests::power_curve_checks_uint_and_float_stats ... ok
test validation::rules::tests::power_curve_skips_rows_with_non_numeric_stats ... ok
test validation::rules::tests::tables_without_both_fields_are_untouched ... ok
test validation::tests::cardinality_many_null_reference_is_a_violation ... ok
test validation::rules::tests::registry_runs_builtin_and_reports_e1601 ... ok
test validation::tests::e1403_escalates_with_warnings_as_errors ... ok
test validation::tests::e1403_three_table_cycle_is_one_warning ... ok
test validation::tests::e1403_self_loop_warns_and_acyclic_is_silent ... ok
test validation::tests::e9006_optional_field_may_be_hidden_silently ... ok
test validation::tests::e9006_primary_key_and_unique_members_hidden_are_conflicts ... ok
test validation::tests::e1403_two_table_cycle_warns_canonically ... ok
test validation::tests::e9006_reference_to_missing_table_is_left_to_the_reference_check ... ok
test validation::tests::e9006_required_field_hidden_by_profile_is_conflict ... ok
test validation::tests::e9006_reference_target_hidden_by_profile_is_conflict ... ok
test registry::tests::push_entry_maps_auth_refusals_and_missing_credentials ... ok
test validation::tests::e9006_required_field_with_default_may_be_hidden ... ok
test validation::tests::e9006_requires_an_active_profile ... ok
test validation::tests::from_str_delegates_to_parse_level ... ok
test validation::tests::l1_ignores_missing_table_when_no_field_is_required ... ok
test validation::tests::l1_errors_on_missing_required_field ... ok
test validation::tests::l1_warns_when_required_table_missing_from_document ... ok
test validation::tests::l1_warns_on_unknown_field_and_escalation_makes_it_an_error ... ok
test validation::tests::l2_map_accepts_valid_keys_and_well_typed_nested_values ... ok
test validation::tests::l2_map_int_keys_must_be_numeric_strings ... ok
test validation::tests::l2_map_non_object_reports_e1101_with_row_location ... ok
test validation::tests::l2_map_rejects_nested_value_mismatch_and_accepts_empty_maps ... ok
test validation::tests::l2_object_schema_still_rejects_extra_keys ... ok
test validation::tests::l2_reports_type_mismatch_e1101 ... ok
test validation::tests::l3_enforces_array_length_bounds ... ok
test validation::tests::l3_enforces_enum_membership ... ok
test validation::tests::l3_enforces_named_enum_membership ... ok
test validation::tests::l3_enforces_numeric_bounds ... ok
test validation::tests::l4_detects_row_ordering_violations ... ok
test validation::tests::l4_skips_pk_check_when_table_has_no_primary_key ... ok
test validation::tests::l5_assert_on_undeclared_target_field_reports_e1004 ... ok
test validation::tests::l5_cardinality_shape_mismatches_report_e1404 ... ok
test validation::tests::l4_detects_duplicate_composite_unique_constraint ... ok
test validation::tests::l5_incomparable_predicate_reports_not_evaluable ... ok
test validation::tests::l5_malformed_reference_assert_reports_e1004 ... ok
test validation::tests::l5_compatible_with_rejects_mismatched_enum_on_target ... ok
test validation::tests::l5_many_cardinality_checks_each_array_element ... ok
test validation::tests::l5_predicate_gate_accepts_when_target_row_satisfies_it ... ok
test validation::tests::l5_unknown_cardinality_spelling_is_a_schema_defect ... ok
test validation::tests::l5_reports_missing_reference_target_e1401 ... ok
test validation::tests::l6_incomparable_operands_report_e1501_with_types ... ok
test validation::tests::l6_malformed_assert_and_unknown_ref_are_schema_defects ... ok
test validation::tests::l3_enforces_string_length_and_pattern ... ok
test validation::tests::l6_schema_defects_report_even_when_the_table_has_no_rows ... ok
test validation::tests::l6_rejects_violating_rule_with_e1501 ... ok
test validation::tests::l6_satisfying_rule_and_absent_optional_field_pass ... ok
test validation::tests::level_all_lists_pipeline_order_and_as_str_matches ... ok
test validation::tests::parse_level_accepts_every_alias_and_rejects_unknown ... ok
test validation::tests::l6_warning_only_rule_reports_a_warning_not_an_error ... ok
test validation::tests::l7_gamerule_level_surfaces_builtin_e1601_end_to_end ... ok
test validation::tests::pipeline_skips_unknown_tables_and_fields_across_levels ... ok
test validation::tests::reference_cache_indexes_pks_and_reports_misses ... ok
test validation::tests::test_value_validation_catches_range ... ok
test validation::tests::validate_with_profile_runs_the_visibility_gate ... ok
test validation::tests::unknown_cardinality_rows_skip_the_reference_check ... ok
test validation::tests::test_duplicate_pk_caught ... ok
test validation::tests::value_matches_type_covers_every_family ... ok
test value::tests::coerce_to_bool_edge_paths ... ok
test value::tests::coerce_to_float_edge_paths ... ok
test value::tests::coerce_to_int_edge_paths ... ok
test value::tests::document_table_construction_and_lookup ... ok
test value::tests::coerce_to_uint_edge_paths ... ok
test value::tests::source_location_line_col_byte_offset_and_fmt ... ok
test value::tests::test_source_location_display ... ok
test value::tests::test_value_coercion ... ok
test value::tests::typed_value_schema_type_attachment ... ok
test value::tests::value_display_for_every_variant ... ok
test value::tests::value_is_empty_classification ... ok
test value::tests::coerce_to_string_edge_paths ... ok
test value::tests::value_type_name_for_every_variant ... ok
test value::tests::value_serde_roundtrip_and_tag_shape ... ok
test registry::tests::push_entry_conflict_refuses_and_dry_run_uploads_nothing ... ok
test registry::tests::sign_verify_roundtrip_tamper_and_untrusted_key_refuse ... ok
test registry::tests::push_entry_presigned_follows_the_same_discipline_without_a_token ... ok
test registry::tests::push_entry_presigned_fresh_package_skips_the_get_and_conflicts_through_it ... ok
test registry::tests::push_entry_uploads_files_then_merged_index ... ok
test registry::tests::push_entry_requires_http_root_and_maps_transport_failure ... ok
test registry::tests::import_bundle_sniffs_zstd_container_and_gates_on_decompressed_entries ... ok
test registry::tests::push_entry_presigned_maps_refusals_to_e2102_and_e2104 ... ok
test registry::tests::export_bundle_zstd_is_byte_deterministic_and_wraps_the_plain_tar ... ok
test registry::tests::push_server_tolerates_misbehaving_clients ... ok

test result: ok. 275 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.36s

     Running unittests src/lib.rs (target/debug/deps/cage_source_csv-3b64b76b0e34be46)

running 20 tests
test tests::test_infer_and_parse_value_boundaries ... ok
test tests::test_parse_csv_string ... ok
test tests::test_parse_csv_flexible ... ok
test tests::test_parse_csv_no_header ... ok
test tests::test_parse_csv_empty_values ... ok
test tests::test_parse_csv_type_inference ... ok
test tests::test_parse_csv_with_header ... ok
test tests::test_parse_file_open_failure ... ok
test tests::test_parse_file_headerless_undecodable_first_row ... ok
test tests::test_parse_file_header_only_yields_empty_table ... ok
test tests::test_parse_file_invalid_utf8_header ... ok
test tests::test_source_adapter_trait ... ok
test tests::test_parse_str_primary_key_falls_back_to_first_field ... ok
test tests::test_parse_file_ragged_row_reports_row_error ... ok
test tests::test_parse_str_trim_disabled_keeps_field_padding ... ok
test tests::test_parse_str_no_header_generates_column_names ... ok
test tests::test_parse_tsv ... ok
test tests::test_parse_file_trim_disabled_keeps_field_padding ... ok
test tests::test_parse_str_ragged_row_reports_error ... ok
test tests::test_source_adapter_trait_parse_delegation ... ok

test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/cage_source_db-ad41657d2d98afb7)

running 36 tests
test draft::tests::cage_kind_strings_cover_the_whole_family ... ok
test draft::tests::not_found_is_an_e1901_lookup_failure ... ok
test draft::tests::render_draft_matches_the_documented_shape_byte_for_byte ... ok
test draft::tests::comments_fire_only_where_a_decision_is_needed ... ok
test draft::tests::yaml_token_quotes_unsafe_names_and_doubles_quotes ... ok
test mysql::tests::classify_is_insensitive_to_case_and_padding ... ok
test mysql::live::live_mysql_bulk_fetch_streams_and_round_trips ... ok
test mysql::tests::draft_sql_passes_the_readonly_whitelist ... ok
test mysql::live::live_mysql_pooled_fetch_round_trips_pins_and_reuses ... ok
test mysql::tests::classify_maps_the_column_type_families ... ok
test pg::live::live_pg_bulk_fetch_round_trips ... ok
test pg::tests::classify_maps_the_data_type_families ... ok
test draft::tests::rendered_draft_parses_back_into_a_schema ... ok
test pg::tests::draft_sql_passes_the_readonly_whitelist ... ok
test mysql::tests::introspect_sql_binds_the_table_name ... ok
test pg::tests::introspect_sql_binds_the_table_name ... ok
test pool::tests::a_failed_connect_propagates_and_retries_next_time ... ok
test pg::tests::classify_reassembles_sized_spelling ... ok
test draft::tests::unusual_names_survive_the_round_trip ... ok
test pg::live::live_pg_pooled_fetch_round_trips_pins_and_reuses ... ok
test pool::tests::mysql_registry_rejects_an_unparsable_dsn_every_time ... ok
test pool::tests::mysql_registry_builds_a_lazy_pool_per_dsn ... ok
test pool::tests::pg_registry_hands_out_one_pool_per_dsn ... ok
test pool::tests::pool_reuses_the_idle_connection_across_uses ... ok
test pool::tests::a_failed_use_discards_the_connection ... ok
test pool::tests::the_idle_stack_is_capped ... ok
test tests::canonical_json_sorts_rows_without_order_by_and_keeps_it_with ... ok
test tests::resolve_query_prefers_named_queries_and_expands_tables ... ok
test tests::resolve_dsn_needs_declared_and_nonempty_env ... ok
test tests::parse_spec_splits_scheme_and_name ... ok
test tests::dbvalue_to_json_maps_types_with_precision_intact ... ok
test tests::validate_select_allows_reads_and_rejects_everything_else ... ok
test tests::no_cache_copy_means_the_transport_error_stands ... ok
test tests::load_rejects_in_order_spec_then_query_then_dsn ... ok
test tests::materialize_round_trips_through_the_cache_file ... ok
test tests::failed_fetch_falls_back_to_the_cache_strict_refuses ... ok

test result: ok. 36 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/cage_source_excel-59713116bcacf939)

running 21 tests
test tests::test_cell_to_cage_value_exotic ... ok
test tests::test_cell_to_cage_value ... ok
test tests::test_cell_to_string_matrix ... ok
test tests::test_col_index_to_letter ... ok
test tests::test_infer_string_type ... ok
test tests::test_resolve_cell_empty_range_keeps_raw_cell ... ok
test tests::test_sanitize_table_name ... ok
test tests::test_resolve_cell_horizontal_and_out_of_range ... ok
test tests::test_open_failure_reports_io_error ... ok
test tests::test_source_adapter_trait ... ok
test tests::test_handbuilt_xlsx_hidden_sheets_are_skipped ... ok
test tests::test_handbuilt_xlsx_bad_merge_ref_still_parses ... ok
test tests::test_handbuilt_xlsx_missing_sheet_part_skips_sheet ... ok
test tests::test_infer_string_type_unsigned ... ok
test tests::test_unsupported_extension_rejected_before_open ... ok
test tests::test_handbuilt_ods_workbook_parses_without_merge_fill ... ok
test tests::test_handbuilt_xlsx_empty_sheet_yields_no_tables ... ok
test tests::test_merged_cells_fill_and_sheet_addressing ... ok
test tests::test_source_adapter_trait_parse_delegation ... ok
test tests::test_handbuilt_xlsx_empty_header_cell_and_blank_row ... ok
test tests::test_has_header_false_generates_column_names ... ok

test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running unittests src/lib.rs (target/debug/deps/cage_source_http-43a586846e71e712)

running 25 tests
test tests::next_page_link_parses_rfc8288_forms ... ok
test tests::page_failure_display_texts ... ok
test tests::non_http_url_is_rejected_without_fetching ... ok
test tests::cache_file_occupied_by_dir_reports_e9902 ... ok
test tests::cache_dir_occupied_by_file_reports_e9902 ... ok
test tests::http_401_maps_to_e1902 ... ok
test tests::http_404_maps_to_e1901 ... ok
test tests::malformed_body_reports_parse_error ... ok
test tests::fetches_parses_and_caches_bytes ... ok
test tests::paginated_page_must_be_an_array ... ok
test tests::rel_param_only_matches_rel_keys ... ok
test tests::read_meta_unreadable_meta_reports_e9902 ... ok
test tests::write_meta_occupied_meta_reports_e9902 ... ok
test tests::pagination_cycle_is_definitive ... ok
test tests::meta_without_payload_is_ignored_and_refetched ... ok
test tests::paginated_pages_merge_in_link_order ... ok
test tests::rate_limit_beyond_cap_is_definitive_and_fast ... ok
test tests::rate_limit_without_header_is_definitive ... ok
test tests::conditional_get_revalidates_and_refreshes ... ok
test tests::deleted_source_and_revoked_access_never_fall_back ... ok
test tests::rate_limit_with_retry_after_is_retried ... ok
test tests::pagination_page_bound_is_enforced ... ok
test tests::dead_port_maps_to_e1901_after_bounded_retries ... ok
test tests::server_tolerates_misbehaving_clients ... ok
test tests::offline_build_falls_back_to_the_cached_copy_strict_refuses ... ok

test result: ok. 25 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.22s

     Running unittests src/lib.rs (target/debug/deps/cage_source_json-9b03f64893302c2a)

running 15 tests
test tests::test_empty_object_yields_fieldless_row ... ok
test tests::test_empty_top_level_array_yields_no_table ... ok
test tests::test_json_value_conversion_edges ... ok
test tests::test_multi_table_null_and_empty_array_skipped ... ok
test tests::test_object_without_id_falls_back_to_first_field ... ok
test tests::test_parse_file_missing_reports_io_error ... ok
test tests::test_helpers_name_sheet_for_nonzero_index ... ok
test tests::test_parse_invalid_json ... ok
test tests::test_root_scalar_rejected ... ok
test tests::test_source_adapter_trait ... ok
test tests::test_parse_json_array ... ok
test tests::test_parse_json_multi_table ... ok
test tests::test_top_level_array_non_object_rows_dropped ... ok
test tests::test_source_adapter_trait_parse_delegation ... ok
test tests::test_parse_json_single_object ... ok

test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/cage_source_msgpack-5dbb2cd66c90a5b7)

running 7 tests
test tests::empty_array_yields_empty_table ... ok
test tests::non_finite_float_reads_back_null ... ok
test tests::shape_violations_error_with_parse_codes ... ok
test tests::plugin_trait_surface ... ok
test tests::numeric_families_reconstruct_exactly ... ok
test tests::stem_names_table_and_pk_heuristic_matches_json ... ok
test tests::target_to_source_round_trip_is_lossless ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/cage_source_sheets-74e27d3b830d347b)

running 11 tests
test tests::build_url_encodes_the_tab_and_hides_the_key_nowhere ... ok
test tests::fetch_error_maps_auth_and_never_leaks_the_key ... ok
test tests::canonical_mapping_follows_the_excel_conventions ... ok
test tests::parse_spec_splits_id_and_tab ... ok
test tests::valid_spreadsheet_id_is_strict ... ok
test tests::shape_gate_rejects_everything_that_is_not_a_row_set ... ok
test tests::offline_transport_failure_falls_back_to_the_cached_copy ... ok
test tests::resolve_credential_needs_declared_and_nonempty_env ... ok
test tests::load_maps_http_and_shape_failures_to_their_codes ... ok
test tests::load_fetches_maps_and_caches_end_to_end ... ok
test tests::load_rejects_spec_and_credential_before_any_network ... ok

test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.36s


running 18 tests
test tests::test_multi_document_error_has_no_line_location ... ok
test tests::test_helpers_name_sheet_for_nonzero_index ... ok
test tests::test_multi_table_scalar_value_skipped ... ok
test tests::test_empty_mapping_yields_fieldless_row ... ok
test tests::test_empty_top_level_sequence_yields_no_table ... ok
test tests::test_root_scalar_rejected ... ok
test tests::test_multi_table_scalar_sequence_skipped ... ok
test tests::test_parse_yaml_sequence ... ok
test tests::test_sequence_of_scalars_dropped ... ok
test tests::test_source_adapter_trait ... ok
test tests::test_scalar_merge_key_rejected ... ok
test tests::test_source_adapter_trait_parse_delegation ... ok
test tests::test_parse_invalid_yaml ... ok
test tests::test_parse_file_missing_reports_io_error ... ok
test tests::test_parse_yaml_single_mapping ... ok
test tests::test_parse_yaml_multi_table ... ok
     Running unittests src/lib.rs (target/debug/deps/cage_source_yaml-27d5676ddf2a6099)
test tests::test_value_and_key_type_matrix ... ok
test tests::test_yaml_anchors_and_aliases ... ok

test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running unittests src/lib.rs (target/debug/deps/cage_target_cpp-5857269ed91cb9a8)

running 22 tests
test tests::test_cpp_uint_literal_small_values_stay_plain ... ok
test tests::test_enum_backing_buckets ... ok
test tests::test_enums_context_empty_slice ... ok
test tests::test_ident_sanitization ... ok
test tests::test_map_default_rendering_rules ... ok
test tests::test_integral_value_literal_fallbacks ... ok
test tests::test_render_default_edge_cases ... ok
test tests::test_from_config_defaults_without_options ... ok
test tests::filter_library_renders_type_and_default ... ok
test tests::test_default_filter_null_without_default ... ok
test tests::test_string_enum_field_maps_to_string_view ... ok
test tests::test_extremes_schema_rendering ... ok
test tests::test_map_field_type_mapping ... ok
test tests::test_field_collisions_get_unique_members ... ok
test tests::test_write_headers_writes_relative_paths ... ok
test tests::test_unresolved_enum_falls_back ... ok
test tests::test_generate_artifact_paths ... ok
test tests::test_table_rendering ... ok
test tests::test_from_config_options ... ok
test tests::test_enums_rendering ... ok
test tests::test_deterministic_output ... ok
test tests::test_generated_headers_compile ... ok

test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.05s

     Running unittests src/lib.rs (target/debug/deps/cage_target_cs-a62e66214d5653b2)

running 19 tests
test tests::test_field_collisions_get_unique_members ... ok
test tests::test_from_config_without_enum_options ... ok
test tests::test_from_config_options ... ok
test tests::test_ident_sanitization ... ok
test tests::test_render_default_edge_cases ... ok
test tests::test_render_default_map_edges ... ok
test tests::filter_library_renders_type_and_default ... ok
test tests::test_schema_without_enums_emits_no_enums_file ... ok
test tests::test_integral_enum_backings ... ok
test tests::test_generate_artifact_paths ... ok
test tests::test_map_default_rendering ... ok
test tests::test_edge_table_types_and_ref_inits ... ok
test tests::test_enums_rendering ... ok
test tests::test_static_class_string_bucket ... ok
test tests::test_empty_primary_key_summary ... ok
test tests::test_table_rendering ... ok
test tests::test_deterministic_output ... ok
test tests::test_unresolved_enum_falls_back_to_string ... ok
test tests::test_map_type_mapping ... ok

test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s

     Running unittests src/lib.rs (target/debug/deps/cage_target_csv-461dc74f44213c9b)

running 15 tests
test tests::test_cage_value_to_string ... ok
test tests::test_cage_value_to_json_all_variants ... ok
test tests::test_cage_value_to_string_edge_values ... ok
test tests::test_from_config_defaults ... ok
test tests::test_from_config_quote_style_always_renders ... ok
test tests::test_empty_table_falls_back_to_primary_key_headers ... ok
test tests::test_from_config_option_overrides ... ok
test tests::test_deterministic_output ... ok
test tests::test_generate_csv ... ok
test tests::test_target_generator_trait ... ok
test tests::test_generate_csv_no_header ... ok
test tests::test_generate_tsv ... ok
test tests::test_generate_row_with_all_value_kinds ... ok
test tests::test_profile_target_filtering ... ok
test tests::test_target_generator_trait_dispatch ... ok

test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/cage_target_flatbuffers-5b938ebe490bd6b3)

running 6 tests
test tests::test_nested_array_and_map_wrappers ... ok
test tests::test_keyword_escaping_and_determinism ... ok
test tests::test_table_header_and_structure ... ok
test tests::test_generate_artifact_paths ... ok
test tests::test_field_mapping_name_order_and_attributes ... ok
test tests::test_enum_base_ladder_and_shared_file ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/cage_target_go-e66209a4b06b2528)

running 22 tests
test tests::dump_sample_output ... ignored, dev-only: dumps sample output for gofmt / go build / go vet checks
test tests::test_enum_value_literal_numeric_fallback_uses_name ... ok
test tests::test_frexp_edges ... ok
test tests::test_doc_finish_flushes_open_section ... ok
test tests::test_from_config_defaults_without_options ... ok
test tests::test_from_config_options ... ok
test tests::test_enum_type_map_allocates_unique_idents ... ok
test tests::test_field_collisions_get_unique_members ... ok
test tests::test_ident_sanitization ... ok
test tests::test_literal_break_terms_oversize_pairs_count_as_zero ... ok
test tests::test_map_default_rendering_rules ... ok
test tests::filter_library_renders_type_and_default ... ok
test tests::test_render_default_edge_cases ... ok
test tests::test_enums_rendering ... ok
test tests::test_write_artifacts ... ok
test tests::test_table_rendering ... ok
test tests::test_map_type_rendering ... ok
test tests::test_package_level_name_collisions ... ok
test tests::test_generate_artifact_paths ... ok
test tests::test_unresolved_enum_falls_back_to_string ... ok
test tests::test_deterministic_output ... ok
test tests::test_sample_schema_rendering ... ok

test result: ok. 21 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.02s

     Running unittests src/lib.rs (target/debug/deps/cage_target_java-5d1859183d6f7de1)

running 25 tests
test tests::test_enum_payload_backing ... ok
test tests::java_default_filter_yields_null_for_a_field_without_default ... ok
test tests::test_enums_context_string_payload_bucket ... ok
test tests::test_from_config_defaults_without_options ... ok
test tests::test_enums_context_skips_unallocated_idents ... ok
test tests::filter_library_renders_type_and_default ... ok
test tests::test_field_collisions_get_unique_members ... ok
test tests::test_ident_sanitization ... ok
test tests::test_render_default_edge_cases ... ok
test tests::write_java_edge_sample_for_javac ... ignored, writes /tmp/cage-java-sample/edge for manual javac runs
test tests::write_java_map_sample_for_javac ... ignored, writes /tmp/cage-java-sample/map for manual javac runs
test tests::write_java_sample_for_javac ... ignored, writes /tmp/cage-java-sample for manual javac runs
test tests::test_map_default_rendering ... ok
test tests::test_class_name_collision ... ok
test tests::test_edge_schema_rendering ... ok
test tests::test_schema_without_enums_emits_no_holder ... ok
test tests::test_enums_file_in_subdirectory ... ok
test tests::test_enums_rendering ... ok
test tests::test_write_sample_artifacts ... ok
test tests::test_generate_artifact_paths ... ok
test tests::test_from_config_options ... ok
test tests::test_map_field_rendering ... ok
test tests::test_unresolved_enum_falls_back_to_string ... ok
test tests::test_table_rendering ... ok
test tests::test_deterministic_output ... ok

test result: ok. 22 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 0.02s

     Running unittests src/lib.rs (target/debug/deps/cage_target_json-e9368057e3533326)

running 13 tests
test tests::test_cage_value_to_json ... ok
test tests::test_cage_value_to_json_numeric_composite_arms ... ok
test tests::test_from_config_option_overrides ... ok
test tests::test_generate_combined_sorted_across_tables ... ok
test tests::test_from_config_defaults ... ok
test tests::test_generate_combined ... ok
test tests::test_generate_json ... ok
test tests::test_deterministic_output ... ok
test tests::test_target_generator_trait ... ok
test tests::test_generate_unsorted_compact_preserves_source_order ... ok
test tests::test_generate_row_with_all_value_kinds ... ok
test tests::test_target_generator_trait_dispatch ... ok
test tests::test_profile_target_filtering ... ok

test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/cage_target_jsonschema-d0942e352f4107a3)

running 11 tests
test tests::test_dangling_enum_degrades_to_string ... ok
test tests::test_composite_types ... ok
test tests::test_unknown_draft_fails_at_generate ... ok
test tests::test_artifact_paths_in_table_name_order ... ok
test tests::test_minimal_table_is_closed_object ... ok
test tests::test_from_config_defaults_and_options ... ok
test tests::test_draft_2020_12 ... ok
test tests::test_document_shape_and_required_list ... ok
test tests::test_enum_fields_are_member_name_strings ... ok
test tests::test_scalar_and_constraint_mapping ... ok
test tests::test_deterministic_bytes_across_runs ... ok

test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/cage_target_lua-1c722740cd4f5a21)

running 19 tests
test tests::test_field_collisions_get_unique_members ... ok
test tests::test_from_config_options ... ok
test tests::test_from_config_without_enum_options ... ok
test tests::test_render_default_edge_cases ... ok
test tests::test_ident_sanitization ... ok
test tests::test_render_default_map_edges ... ok
test tests::filter_library_renders_type_and_default ... ok
test tests::test_empty_table_without_primary_key ... ok
test tests::test_schema_without_enums_emits_no_enums_file ... ok
test tests::test_map_defaults_render ... ok
test tests::test_generate_artifact_paths ... ok
test tests::test_enum_description_and_mixed_value_arms ... ok
test tests::test_map_type_labels_and_doc_notes ... ok
test tests::test_table_rendering ... ok
test tests::test_type_labels_and_bool_defaults ... ok
test tests::test_map_default_copy_runs_under_lua ... ok
test tests::test_enums_rendering ... ok
test tests::test_unresolved_enum_falls_back_to_string ... ok
test tests::test_deterministic_output ... ok

test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s

     Running unittests src/lib.rs (target/debug/deps/cage_target_msgpack-7fa9ce5463f900cf)

running 8 tests
test tests::test_from_config_defaults ... ok
test tests::test_from_config_option_overrides ... ok
test tests::test_deterministic_output ... ok
test tests::test_generate_golden_bytes ... ok
test tests::test_target_generator_trait ... ok
test tests::test_generate_row_with_all_value_kinds ... ok
test tests::test_sort_keys_false_keeps_source_order ... ok
test tests::test_profile_target_filtering ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/lib.rs (target/debug/deps/cage_target_proto-e9def1f94813ab7e)

running 14 tests
test tests::test_enum_out_of_int32_range_falls_back_to_string ... ok
test tests::test_deterministic_output ... ok
test tests::test_ident_sanitization_and_dedupe ... ok
test tests::test_empty_table_message ... ok
test tests::test_from_config_options ... ok
test tests::test_schema_hash_optional ... ok
test tests::test_nested_wrappers_for_arrays_and_maps ... ok
test tests::test_wellknown_import_only_when_used ... ok
test tests::test_enum_zero_present_and_aliases ... ok
test tests::test_table_header_and_structure ... ok
test tests::test_shared_enums_file ... ok
test tests::test_field_mapping_name_order_and_numbers ... ok
test tests::test_enums_file_option_drives_import ... ok
test tests::test_generate_artifact_paths ... ok

test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running unittests src/lib.rs (target/debug/deps/cage_target_py-a059a7ea5a90b463)

running 19 tests
test tests::test_field_collisions_get_unique_members ... ok
test tests::test_from_config_options ... ok
test tests::filter_library_renders_type_and_default ... ok
test tests::test_render_default_edge_cases ... ok
test tests::test_ident_sanitization ... ok
test tests::test_render_default_map_edge_cases ... ok
test tests::test_from_config_without_enum_options ... ok
test tests::test_optional_only_table_skips_plain_group ... ok
test tests::test_schema_without_enums_emits_no_enums_file ... ok
test tests::test_enums_rendering ... ok
test tests::test_enum_description_and_mixed_string_bucket ... ok
test tests::test_generate_artifact_paths ... ok
test tests::test_empty_table_renders_pass_stub ... ok
test tests::test_any_bytes_object_and_bool_defaults ... ok
test tests::test_map_type_annotations ... ok
test tests::test_deterministic_output ... ok
test tests::test_map_defaults ... ok
test tests::test_table_rendering ... ok
test tests::test_unresolved_enum_falls_back_to_str ... ok

test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s

     Running unittests src/lib.rs (target/debug/deps/cage_target_template-2560d30bad233bdd)

running 13 tests
test tests::empty_template_dir_is_an_error ... ok
test tests::extended_convention_filters_and_field_order ... ok
test tests::global_template_renders_once_with_ir_exposed ... ok
test tests::field_descriptions_and_defaults_are_referencable ... ok
test tests::from_config_reads_template_dir_option ... ok
test tests::words_split_on_case_digit_and_separator_boundaries ... ok
test tests::missing_template_dir_is_an_error ... ok
test tests::broken_template_syntax_is_an_error_naming_the_template ... ok
test tests::generate_with_setup_mounts_extra_filters ... ok
test tests::schema_hash_is_exposed_to_templates ... ok
test tests::naming_filters_convert_identifiers ... ok
test tests::official_mode_uses_in_memory_templates_extras_and_filters ... ok
test tests::per_table_template_renders_every_table_in_name_order ... ok

test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running unittests src/lib.rs (target/debug/deps/cage_target_ts-aafd4a90c49fb0ea)

running 30 tests
test tests::test_dts_path ... ok
test tests::test_field_collisions_get_unique_members ... ok
test tests::test_from_config_options ... ok
test tests::test_ident_sanitization ... ok
test tests::test_jsdoc_desc_safety ... ok
test tests::filter_library_renders_type_and_default ... ok
test tests::test_empty_defaults_still_emit_members ... ok
test tests::test_map_defaults_new_map_ts ... ok
test tests::test_javascript_enums_and_dts ... ok
test tests::test_map_default_mismatch_skipped ... ok
test tests::test_map_type_and_default_units ... ok
test tests::test_module_specifier ... ok
test tests::test_render_default_edge_cases ... ok
test tests::test_map_dts_declarations ... ok
test tests::test_ts_type_mapping ... ok
test tests::test_length_pattern_constraints_in_member_docs ... ok
test tests::write_sample_output ... ignored, writes /tmp/cage-ts-sample for tsc/node sanity checks
test tests::test_enums_rendering ... ok
test tests::test_generate_artifact_paths ... ok
test tests::test_map_types_ts ... ok
test tests::test_property_keys_stay_unescaped ... ok
test tests::test_javascript_table_rendering ... ok
test tests::test_enum_description_and_mixed_value_buckets ... ok
test tests::test_unresolved_enum_falls_back ... ok
test tests::test_map_jsdoc_typedef ... ok
test tests::test_empty_table_renders_bare_interface ... ok
test tests::test_javascript_artifact_order ... ok
test tests::test_table_rendering ... ok
test tests::test_import_alias_on_local_collision ... ok
test tests::test_deterministic_output ... ok

test result: ok. 29 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.13s

   Doc-tests cage_core

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_source_csv

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_source_db

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_source_excel

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_source_http

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_source_json

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_source_msgpack

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_source_sheets

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_source_yaml

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_target_cpp

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_target_cs

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_target_csv

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_target_flatbuffers

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_target_go

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_target_java

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_target_json

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_target_jsonschema

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_target_lua

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_target_msgpack

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_target_proto

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_target_py

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_target_template

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cage_target_ts

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s



[showing lines 424-1395 of 1395; full output saved to /home/cui/.local/share/opencode/shell/5c5b7fc368d18b4d9c5cf962a08a52154ffcde08/sh_14e5537d3001VCiN5L9FHKhbGa.out]
[0m$ [0mnpm run docs:build
> cage@0.1.0 docs:build
> vitepress build docs


  vitepress 2.0.0-alpha.20  (using vite 8.3.2)

- building client + server bundles...
[plugin builtin:vite-reporter] 
(!) Some chunks are larger than 500 kB after minification. Consider:
- Using dynamic import() to code-split the application
- Use build.rolldownOptions.output.codeSplitting to improve chunking: https://rolldown.rs/reference/OutputOptions.codeSplitting
- Adjust chunk size limit for this warning via build.chunkSizeWarningLimit.
[32m✓[0m building client + server bundles...
- rendering pages...
[32m✓[0m rendering pages...
build complete in 9.22s.
> build · gpt-6-luna
[0m✱ [0mGrep "predicate|compatible_with" [90min crates/cage-core/src/validation/mod.rs · 52 matches[0m
[0m✱ [0mGrep "streaming|query_iter|buffer|100,000|10 万|simple protocol" [90min docs/design.md · 16 matches[0m
[0m✱ [0mGrep "dynamic library|C ABI|in-process|不可信代码|Plugin SDK" [90min todo.md · 1 match[0m
[0m→ [0mRead crates/cage-core/src/validation/mod.rs [90m[offset=935, limit=75][0m
[0m→ [0mRead crates/cage-cli/src/web.rs [90m[offset=114, limit=45][0m
> build · gpt-6-luna
## 1) 最薄弱的 3 处（按风险排序）

1. **本地 Web 服务的写操作缺少请求来源校验和请求体上限** — `crates/cage-cli/src/web.rs:60, 96-105, 119-145`
   - 服务虽只绑定 `127.0.0.1`，但无鉴权；处理 POST 时直接读完整请求体，未见大小限制，也未校验 `Origin` / `Host`。
   - `/api/schema` 会写 Schema（`web.rs:372-375`），`/api/build` 和 `/api/snapshot` 会触发构建、写产物。若浏览器允许页面向 loopback 发请求，恶意网页可尝试跨站触发这些操作；超大请求体还可能耗尽内存。单线程请求循环（`web.rs:73-75`）会放大阻塞影响。
   - 建议至少加请求体上限、Host/Origin 校验和 CSRF 防护；本地服务定位不能替代这些边界检查。

2. **快照校验只证明“内容与随包账本一致”，不证明来源可信** — `crates/cage-core/src/snapshot/mod.rs:101-128, 149-174`
   - `verify_snapshot` 把快照内的 `HASHES.json` 当信任根；攻击者若能同时改产物和账本中的哈希，校验仍可能通过。`load` 也没有外部可信密钥校验。
   - 这适合发现意外损坏或单独篡改，但不等同于防恶意篡改。签名目前用于 bundle，且导入时可选要求验签（`docs/cli.md:420-421`）；运行时快照没有同等的签名信任链。跨机器或不可信存储分发时需明确这一限制。

3. **PostgreSQL 全量结果缓冲，缺少规模上限** — `crates/cage-source-db/src/pg.rs:160-190`
   - `simple_query` 先收集完整响应，再收集全部行并转换成另一份 `RowSet`，大表会产生较高峰值内存；仓库明确实测了 10 万行，但没有看到硬限制或流式回压。
   - 设计文档已承认 PostgreSQL 当前维持缓冲（`docs/design.md:3135-3143`）。风险主要在用户将它用于远超配置数据预期的大表时。

## 2) 最值得补的 3 项新能力

1. **可验签的 Snapshot**
   - 将签名覆盖扩展到 snapshot，并让运行时加载可配置可信公钥、支持强制验签。
   - 现在没有：现有信任模型是快照自带哈希账本；签名只落在 registry bundle 分发链路。补上后才能把“完整性校验”提升为“来源认证”。

2. **LSP 内联定位与基于 Schema 的编辑能力**
   - 将文本源诊断映射到真实行列范围，再逐步加入补全、悬停或跳转定义。
   - 现在没有：LSP 明确只做诊断，使用零宽文件级位置，并校验磁盘状态而非编辑缓冲（`crates/cage-cli/src/lsp.rs:14-24`；`docs/cli.md:379-395`）。要实现需要把编辑器缓冲接入校验，并建立稳定的 Schema 语义查询接口。

3. **PostgreSQL 流式读取及资源预算**
   - 减少整表多份驻留，并增加最大行数/字节数等可配置保护。
   - 现在没有：当前实现基于同步 PostgreSQL simple-query 路径整体缓冲；设计选择是配置数据规模下先接受该成本，且缺少可作为门禁的超大表性能基线（`docs/design.md:3135-3143`）。

## 3) 文档与实现可能对不上的地方

- **迁移场景命令示例已过期**：`docs/lifecycle.md:132-133` 写 `cage schema-draft --db ...` 和 `cage migrate-draft . --out ...`。实际 `schema-draft` 要项目路径和 `mysql:<table>` / `pg:<table>` spec；`migrate-draft` 要两个 schema 文件及 `--from`、`--to`，见 `crates/cage-cli/src/main.rs:292-327`。
- **环境化 Snapshot / Registry 状态自相矛盾**：`docs/architecture.md:211` 仍称环境化出包“留待”；同文件 `:213` 已描述 Snapshot `--env`，而 `docs/cli.md:431-432` 也写明 Registry 发布支持 `--env`。英文版 `docs/architecture.en.md:207-209` 有同样旧口径。
- **IDE 状态未同步**：`docs/validation.md:479` 说 IDE 插件留待后续；仓库已有 `cage lsp` 和 VSCode 扩展，参见 `docs/cli.md:367-407`。
- **引用谓词状态未同步**：`README.zh.md:40` 把语义谓词校验写成预留；实际 L5 已解析并执行谓词，见 `crates/cage-core/src/validation/mod.rs:885-985, 1195-1216`。
- **“校准”措辞易误导**：`docs/requirements.md:18` 称 CSV 类型推断后由 Schema 在 L2“严格校准”；`docs/source.md:22` 则说明 L2 是严格类型校验、不做跨类型重校准。建议统一成“严格校验”，避免让用户以为会自动转换类型。

**门禁**：`cargo test --workspace --locked` 通过；`npm run docs:build` 通过，VitePress 有一个大于 500 kB 的 chunk 警告。
