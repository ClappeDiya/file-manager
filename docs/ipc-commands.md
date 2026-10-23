# Tauri IPC Commands

> Auto-generated from `src-tauri/src/commands/*.rs` by
> `scripts/dump-ipc-commands.sh`. **Do not edit by hand** —
> re-run `pnpm docs:ipc` after changing the command surface.

This is the canonical list of every `#[tauri::command]` callable
from the React/TypeScript frontend via
`tauriInvoke<T>("command_name", { args })` (see
`src/hooks/use-tauri.ts`) or its fallback-safe variant
`tauriInvokeSafe<T>()`.

Each command is registered in `src-tauri/src/lib.rs`'s `invoke_handler`
macro; if you add a new one here, register it there too.

## `ai_commands` (20 commands)

- `ai_accept_suggestion`
- `ai_activity_report`
- `ai_chat`
- `ai_check_confirmation_needed`
- `ai_clear_chat`
- `ai_confirm_action`
- `ai_dismiss_suggestion`
- `ai_execute_parsed_job`
- `ai_explain_error`
- `ai_generate_suggestions`
- `ai_get_audit_log`
- `ai_get_chat_history`
- `ai_get_feature_toggles`
- `ai_get_ollama_model`
- `ai_get_suggestions`
- `ai_help`
- `ai_parse_natural_language`
- `ai_probe_ollama`
- `ai_set_feature_toggles`
- `ai_set_ollama_model`

## `archive_commands` (4 commands)

- `archive_browse`
- `archive_create`
- `archive_extract`
- `archive_info`

## `audit_commands` (4 commands)

- `create_audit_anchor`
- `export_audit_log`
- `get_audit_log_status`
- `verify_audit_log`

## `automation_commands` (21 commands)

- `approve_automation_command`
- `automation_convert_to_workflow`
- `automation_restore_revision`
- `automation_rule_revisions`
- `clear_automation_logs`
- `clone_automation_rule`
- `create_automation_rule`
- `delete_automation_rule`
- `enable_automation_rule`
- `get_automation_rule`
- `list_automation_logs`
- `list_automation_rules`
- `parse_automation_nl`
- `pin_ledger_event`
- `quickflow_suggestion_accept`
- `quickflow_suggestion_dismiss`
- `quickflow_suggestions_list`
- `retry_failed_event`
- `rotate_automation_webhook`
- `run_automation_rule`
- `update_automation_rule`

## `aws_commands` (6 commands)

- `assume_role_with_web_identity`
- `get_sso_role_credentials`
- `list_aws_profiles`
- `poll_sso_token`
- `resolve_aws_profile`
- `start_sso_auth`

## `backup_commands` (98 commands)

- `backup_adopt_repository`
- `backup_apply_template`
- `backup_break_lock`
- `backup_browse_remote`
- `backup_bug_report`
- `backup_cache_stats`
- `backup_cancel`
- `backup_capabilities`
- `backup_change_passphrase`
- `backup_cleanup_databases`
- `backup_clear_cache`
- `backup_close_drive`
- `backup_cloud_providers`
- `backup_compare`
- `backup_crypt_file`
- `backup_database_inventory`
- `backup_decrypt_volume`
- `backup_delete_destination`
- `backup_delete_job`
- `backup_delete_template`
- `backup_diagnose`
- `backup_direct_restore`
- `backup_discover_remote_config`
- `backup_effective_caps`
- `backup_effective_job`
- `backup_effective_settings`
- `backup_estimate`
- `backup_execute`
- `backup_export_cli`
- `backup_export_config`
- `backup_export_file`
- `backup_export_is_encrypted`
- `backup_export_templates`
- `backup_generate_passphrase`
- `backup_get_cache_settings`
- `backup_get_desktop_settings`
- `backup_get_global_defaults`
- `backup_get_job`
- `backup_get_secret_settings`
- `backup_get_template`
- `backup_has_destination_secret`
- `backup_import_config`
- `backup_import_templates`
- `backup_job_placeholders`
- `backup_list_destinations`
- `backup_list_drives`
- `backup_list_files`
- `backup_list_jobs`
- `backup_list_sources`
- `backup_list_templates`
- `backup_list_versions`
- `backup_lock_status`
- `backup_open_drive`
- `backup_open_point_in_time`
- `backup_passphrase_strength`
- `backup_pause`
- `backup_photos_libraries`
- `backup_photos_library_info`
- `backup_place_legal_hold`
- `backup_preview_file`
- `backup_preview_retention`
- `backup_preview_sources`
- `backup_prune_history`
- `backup_read_file`
- `backup_recovery_convert`
- `backup_recovery_download`
- `backup_recovery_index`
- `backup_recovery_list`
- `backup_recovery_restore`
- `backup_release_legal_hold`
- `backup_remote_delete`
- `backup_remote_get`
- `backup_remote_list`
- `backup_remote_put`
- `backup_resolve_version`
- `backup_resume`
- `backup_run`
- `backup_run_all`
- `backup_run_history`
- `backup_save_destination`
- `backup_save_job`
- `backup_save_template`
- `backup_schedule_preview`
- `backup_search`
- `backup_set_cache_settings`
- `backup_set_database_encryption`
- `backup_set_desktop_settings`
- `backup_set_destination_secret`
- `backup_set_global_defaults`
- `backup_set_secret_settings`
- `backup_set_version_metadata`
- `backup_start_operation`
- `backup_status`
- `backup_test_destination`
- `backup_test_destination_compat`
- `backup_test_notification`
- `backup_test_secret_providers`
- `ediscovery_export`

## `batch_rename_commands` (4 commands)

- `batch_rename_apply`
- `batch_rename_preview`
- `batch_rename_undo`
- `naming_preview`

## `confirmation_commands` (1 command)

- `request_destructive_confirmation`

## `connection_commands` (17 commands)

- `create_connection_group`
- `delete_connection`
- `delete_connection_group`
- `export_connections`
- `get_connection`
- `get_global_proxy`
- `import_connections`
- `import_from_third_party`
- `list_connection_groups`
- `list_connections`
- `list_ssh_agents`
- `list_ssh_config_hosts`
- `resolve_ssh_config`
- `save_connection`
- `search_connections`
- `set_global_proxy`
- `test_connection`

## `connector_commands` (11 commands)

- `connection_presets`
- `connector_connect`
- `connector_disconnect`
- `connector_download`
- `connector_is_connected`
- `connector_list_protocols`
- `connector_list_remote`
- `connector_make_directory`
- `connector_remove`
- `connector_upload`
- `rclone_list_remotes`

## `convert_commands` (3 commands)

- `convert_capabilities`
- `convert_documents`
- `convert_images`

## `custom_commands` (6 commands)

- `delete_custom_command`
- `execute_custom_command`
- `expand_custom_command`
- `generate_script`
- `list_custom_commands`
- `save_custom_command`

## `drive_commands` (6 commands)

- `detect_drives`
- `eject_drive`
- `nfs_list_exports`
- `smb_discover_shares`
- `smb_list_shares`
- `transfer_preflight`

## `editor_commands` (11 commands)

- `check_watched_files`
- `delete_editor_mapping`
- `get_double_click_behavior`
- `list_editor_mappings`
- `list_watched_files`
- `open_in_editor`
- `resolve_editor`
- `save_editor_mapping`
- `set_double_click_behavior`
- `start_file_watch`
- `stop_file_watch`

## `editor_draft_commands` (4 commands)

- `editor_draft_discard`
- `editor_draft_list`
- `editor_draft_load`
- `editor_draft_save`

## `encryption_commands` (20 commands)

- `add_encryption_policy`
- `change_vault_password`
- `check_encryption_policy`
- `check_transport_security`
- `check_transport_security_custom`
- `create_vault`
- `decrypt_from_download`
- `encrypt_for_upload`
- `enforce_https_url`
- `get_recommended_security`
- `get_unlocked_vault_count`
- `is_vault_unlocked`
- `list_encryption_policies`
- `list_vaults`
- `lock_vault`
- `remove_encryption_policy`
- `tls_fetch_fingerprint`
- `unlock_vault`
- `vault_decrypt_file`
- `vault_encrypt_file`

## `file_ops_commands` (26 commands)

- `cloud_delete_permanently`
- `cloud_delete_to_trash`
- `copy_files`
- `copy_files_to_many`
- `copy_path`
- `copy_relative_path`
- `copy_remote_path`
- `copy_url`
- `count_affected_files`
- `create_directory`
- `create_file`
- `create_symlink`
- `delete_files`
- `duplicate_files`
- `ensure_directory`
- `get_file_metadata`
- `move_files`
- `open_file_with_default`
- `read_symlink`
- `redo_file_operation`
- `rename_file`
- `resolve_path`
- `reveal_in_os`
- `set_owner`
- `set_permissions`
- `undo_file_operation`

## `fleet_agent_commands` (13 commands)

- `fleet_agent_cancel_claim`
- `fleet_agent_connect_cancel`
- `fleet_agent_connect_finish`
- `fleet_agent_connect_start`
- `fleet_agent_get_limits`
- `fleet_agent_poll_claim`
- `fleet_agent_register`
- `fleet_agent_set_enabled`
- `fleet_agent_set_limits`
- `fleet_agent_start_claim`
- `fleet_agent_status`
- `fleet_agent_sync_now`
- `fleet_agent_unregister`

## `fs_commands` (6 commands)

- `list_directory`
- `pick_files`
- `pick_folder`
- `pick_save_file`
- `read_text_file_full`
- `write_text_file_full`

## `gpg_commands` (10 commands)

- `gpg_decrypt_files`
- `gpg_encrypt_files`
- `gpg_export_public`
- `gpg_generate_key`
- `gpg_import_keys`
- `gpg_list_keys`
- `gpg_recrypt_files`
- `gpg_sign_files`
- `gpg_status`
- `gpg_verify`

## `integrity_commands` (25 commands)

- `integrity_checksum`
- `integrity_create_smart_folder`
- `integrity_create_tag`
- `integrity_delete_smart_folder`
- `integrity_delete_tag`
- `integrity_find_duplicates`
- `integrity_get_file_info`
- `integrity_list_smart_folders`
- `integrity_list_tags`
- `integrity_metadata_keys`
- `integrity_remove_label`
- `integrity_remove_metadata`
- `integrity_resolve_duplicates`
- `integrity_run_smart_folder`
- `integrity_search_metadata`
- `integrity_set_label`
- `integrity_set_metadata`
- `integrity_tag_file`
- `integrity_untag_file`
- `integrity_verify`
- `metadata_check_compliance`
- `metadata_delete_schema`
- `metadata_list_schemas`
- `metadata_save_schema`
- `metadata_schema_for`

## `ledger_commands` (10 commands)

- `ledger_count`
- `ledger_directory_activity`
- `ledger_extension_destinations`
- `ledger_frecent_paths`
- `ledger_get_pulse`
- `ledger_prune`
- `ledger_query`
- `ledger_recent`
- `ledger_recent_paths`
- `ledger_since_last_seen`

## `lineage_commands` (1 command)

- `get_file_lineage`

## `log_commands` (13 commands)

- `add_trusted_host_ca`
- `check_gssapi_available`
- `diagnostics_create`
- `diagnostics_send`
- `export_logs_json`
- `export_logs_to_file`
- `get_log_directory`
- `get_log_level`
- `get_log_retention`
- `list_trusted_host_cas`
- `remove_trusted_host_ca`
- `set_log_level`
- `set_log_retention`

## `master_password_commands` (9 commands)

- `change_master_password`
- `is_app_unlocked`
- `is_master_password_set`
- `master_decrypt_credential`
- `master_encrypt_credential`
- `master_lock_app`
- `master_unlock_app`
- `set_master_password`
- `verify_master_password`

## `mount_commands` (11 commands)

- `check_mount_capability`
- `delete_mount_config`
- `get_mount_status`
- `list_mounts`
- `mount_cache_usage`
- `mount_clear_cache`
- `mount_pin_offline`
- `mount_policy`
- `mount_remote`
- `save_mount_config`
- `unmount_remote`

## `narrator_commands` (1 command)

- `narrator_narrate_operation`

## `network_wizard_commands` (5 commands)

- `network_run_full_diagnostic`
- `network_test_auth`
- `network_test_dns`
- `network_test_port`
- `network_test_tls`

## `notification_commands` (3 commands)

- `get_notification_prefs`
- `send_notification`
- `set_notification_prefs`

## `operations_commands` (2 commands)

- `operations_cancel`
- `operations_snapshot`

## `peer_commands` (33 commands)

- `peer_cancel_transfer`
- `peer_clear_finished`
- `peer_connect_manual`
- `peer_get_peer`
- `peer_get_transfer_progress`
- `peer_is_discovering`
- `peer_list_online`
- `peer_list_peers`
- `peer_list_pending_requests`
- `peer_list_saved`
- `peer_list_transfers`
- `peer_local_info`
- `peer_remove_saved`
- `peer_request_transfer`
- `peer_respond_transfer`
- `peer_save_peer`
- `peer_set_display_name`
- `peer_set_trust`
- `peer_start_discovery`
- `peer_start_listener`
- `peer_stop_discovery`
- `peer_stop_listener`
- `server_transfer_cancel`
- `server_transfer_capability_matrix`
- `server_transfer_cleanup`
- `server_transfer_create`
- `server_transfer_get`
- `server_transfer_list`
- `server_transfer_list_active`
- `server_transfer_pause`
- `server_transfer_preview_method`
- `server_transfer_retry`
- `server_transfer_start`

## `policy_commands` (3 commands)

- `filename_policies_list`
- `filename_policies_save`
- `filename_policy_check`

## `preview_commands` (3 commands)

- `preview_file`
- `preview_get_exif`
- `preview_office_render`

## `proxy_commands` (4 commands)

- `managed_settings_get`
- `proxy_get`
- `proxy_set`
- `proxy_test`

## `s3_commands` (18 commands)

- `s3_create_invalidation`
- `s3_delete_lifecycle_rule`
- `s3_delete_object_tags`
- `s3_delete_with_mfa`
- `s3_get_bucket_settings`
- `s3_get_lifecycle_rules`
- `s3_get_mfa_delete_status`
- `s3_get_object_properties`
- `s3_get_object_tags`
- `s3_list_distributions`
- `s3_list_endpoint_presets`
- `s3_list_invalidations`
- `s3_put_lifecycle_rule`
- `s3_put_object_tags`
- `s3_set_object_acl`
- `s3_set_storage_class`
- `s3_set_transfer_acceleration`
- `s3_validate_endpoint`

## `safety_commands` (2 commands)

- `safety_assess_intent`
- `safety_confirm_intent`

## `schedule_commands` (8 commands)

- `calendar_delete`
- `calendar_import_ics`
- `calendar_save`
- `calendars_list`
- `schedule_delete`
- `schedule_preview`
- `schedule_save`
- `schedules_list`

## `search_commands` (6 commands)

- `search_files_query`
- `search_index_add`
- `search_index_cancel`
- `search_index_refresh`
- `search_index_remove`
- `search_index_roots`

## `settings_commands` (11 commands)

- `export_all_settings`
- `get_privacy_settings`
- `get_settings_summary`
- `get_settings_sync_dir`
- `get_settings_sync_status`
- `import_all_settings`
- `set_privacy_settings`
- `set_settings_auto_sync`
- `set_settings_sync_dir`
- `sync_settings_from_dir`
- `sync_settings_to_dir`

## `share_commands` (3 commands)

- `connector_create_share_link`
- `sharing_policy_get`
- `sharing_policy_set`

## `space_commands` (9 commands)

- `activate_space`
- `attach_space_automation`
- `create_space`
- `delete_space`
- `detach_space_automation`
- `get_space`
- `get_space_status`
- `list_spaces`
- `update_space`

## `ssh_key_commands` (4 commands)

- `forget_known_host`
- `generate_ssh_key`
- `list_known_hosts`
- `list_ssh_keys`

## `state_commands` (6 commands)

- `get_config`
- `list_config`
- `load_workspace_state`
- `reset_database`
- `save_workspace_state`
- `set_config`

## `sync_commands` (26 commands)

- `cancel_sync`
- `create_sync_pair`
- `delete_sync_pair`
- `detect_time_offset`
- `export_sync_preview_csv`
- `export_sync_report_csv`
- `export_sync_report_json`
- `get_quarantine_entries`
- `get_sync_conflicts`
- `get_sync_filter_presets`
- `get_sync_health`
- `get_sync_progress`
- `get_sync_reports`
- `get_sync_run_history`
- `list_sync_pairs`
- `parse_robocopy_command`
- `prune_sync_run_history`
- `resolve_sync_conflict`
- `robocopy_command_preview`
- `rollback_sync`
- `run_sync`
- `start_sync_watcher`
- `stop_sync_watcher`
- `sync_dry_run`
- `update_sync_pair`
- `validate_sync_cron`

## `system_commands` (3 commands)

- `get_app_version`
- `get_platform_info`
- `greet`

## `terminal_commands` (11 commands)

- `terminal_close_session`
- `terminal_create_local`
- `terminal_create_remote`
- `terminal_escape_path`
- `terminal_get_default_shell`
- `terminal_get_layout`
- `terminal_list_sessions`
- `terminal_read_scrollback`
- `terminal_resize`
- `terminal_set_layout`
- `terminal_write`

## `thumbnail_commands` (2 commands)

- `thumbnail_cache_clear`
- `thumbnail_get`

## `transfer_commands` (35 commands)

- `cancel_transfer`
- `check_conflict`
- `cleanup_transfer_history`
- `complete_transfer`
- `compute_checksum`
- `enqueue_transfer`
- `export_transfer_history`
- `fail_transfer`
- `get_completion_hooks`
- `get_throttle_config`
- `get_transfer`
- `get_transfer_config`
- `get_verify_tier`
- `list_failed_transfers`
- `list_transfers`
- `pause_transfer`
- `reorder_transfer`
- `resolve_conflict`
- `resume_transfer`
- `retry_all_failed`
- `retry_transfer`
- `search_transfer_history`
- `set_checksum_algorithm`
- `set_completion_hooks`
- `set_conflict_policy`
- `set_connection_throttle`
- `set_global_throttle`
- `set_retry_policy`
- `set_transfer_config`
- `set_transfer_conflict_policy`
- `set_transfer_priority`
- `set_verify_checksums`
- `set_verify_tier`
- `update_transfer_progress`
- `verify_transfer`

## `undo_commands` (6 commands)

- `list_redoable`
- `list_undoable`
- `redo_by_correlation`
- `redo_last`
- `undo_by_correlation`
- `undo_last`

## `url_handler_commands` (4 commands)

- `handle_incoming_url`
- `is_url_handler_registered`
- `register_url_handlers`
- `unregister_url_handlers`

## `version_commands` (2 commands)

- `list_file_versions`
- `restore_file_version`

---

**Surface:** 570 commands across 51 modules.
