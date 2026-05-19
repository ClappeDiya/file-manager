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

## `ai_commands` (18 commands)

- `ai_accept_suggestion`
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
- `ai_parse_natural_language`
- `ai_probe_ollama`
- `ai_set_feature_toggles`
- `ai_set_ollama_model`

## `archive_commands` (4 commands)

- `archive_browse`
- `archive_create`
- `archive_extract`
- `archive_info`

## `automation_commands` (16 commands)

- `clear_automation_logs`
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
- `run_automation_rule`
- `test_automation_rule`
- `update_automation_rule`

## `aws_commands` (6 commands)

- `assume_role_with_web_identity`
- `get_sso_role_credentials`
- `list_aws_profiles`
- `poll_sso_token`
- `resolve_aws_profile`
- `start_sso_auth`

## `batch_rename_commands` (3 commands)

- `batch_rename_apply`
- `batch_rename_preview`
- `batch_rename_undo`

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

## `connector_commands` (5 commands)

- `connector_connect`
- `connector_disconnect`
- `connector_is_connected`
- `connector_list_protocols`
- `connector_list_remote`

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

## `encryption_commands` (19 commands)

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
- `unlock_vault`
- `vault_decrypt_file`
- `vault_encrypt_file`

## `file_ops_commands` (24 commands)

- `cloud_delete_permanently`
- `cloud_delete_to_trash`
- `copy_files`
- `copy_path`
- `copy_relative_path`
- `copy_remote_path`
- `copy_url`
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

## `fs_commands` (4 commands)

- `list_directory`
- `pick_folder`
- `read_text_file_full`
- `write_text_file_full`

## `integrity_commands` (16 commands)

- `integrity_checksum`
- `integrity_create_smart_folder`
- `integrity_create_tag`
- `integrity_delete_smart_folder`
- `integrity_delete_tag`
- `integrity_find_duplicates`
- `integrity_get_file_info`
- `integrity_list_smart_folders`
- `integrity_list_tags`
- `integrity_remove_label`
- `integrity_resolve_duplicates`
- `integrity_run_smart_folder`
- `integrity_set_label`
- `integrity_tag_file`
- `integrity_untag_file`
- `integrity_verify`

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

## `log_commands` (9 commands)

- `add_trusted_host_ca`
- `check_gssapi_available`
- `export_logs_json`
- `export_logs_to_file`
- `get_log_directory`
- `get_log_level`
- `list_trusted_host_cas`
- `remove_trusted_host_ca`
- `set_log_level`

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

## `mount_commands` (7 commands)

- `check_mount_capability`
- `delete_mount_config`
- `get_mount_status`
- `list_mounts`
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

## `peer_commands` (29 commands)

- `peer_cancel_transfer`
- `peer_connect_manual`
- `peer_get_peer`
- `peer_get_transfer_progress`
- `peer_is_discovering`
- `peer_list_online`
- `peer_list_peers`
- `peer_list_pending_requests`
- `peer_list_saved`
- `peer_list_transfers`
- `peer_remove_saved`
- `peer_request_transfer`
- `peer_respond_transfer`
- `peer_save_peer`
- `peer_set_display_name`
- `peer_set_trust`
- `peer_start_discovery`
- `peer_stop_discovery`
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

## `preview_commands` (2 commands)

- `preview_file`
- `preview_get_exif`

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

## `ssh_key_commands` (2 commands)

- `generate_ssh_key`
- `list_ssh_keys`

## `state_commands` (5 commands)

- `get_config`
- `load_workspace_state`
- `reset_database`
- `save_workspace_state`
- `set_config`

## `sync_commands` (20 commands)

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
- `get_sync_reports`
- `list_sync_pairs`
- `resolve_sync_conflict`
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

## `terminal_commands` (10 commands)

- `terminal_close_session`
- `terminal_create_local`
- `terminal_create_remote`
- `terminal_escape_path`
- `terminal_get_default_shell`
- `terminal_get_layout`
- `terminal_list_sessions`
- `terminal_resize`
- `terminal_set_layout`
- `terminal_write`

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

**Surface:** 359 commands across 38 modules.
