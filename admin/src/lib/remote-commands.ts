/**
 * Client API calls a remote session may make: job management, runs and
 * maintenance. Calls that read secrets back or touch arbitrary local files
 * (config export with secrets, remote file get/put, file encryption) are excluded.
 */
export const RPC_COMMANDS = [
  'backup_list_jobs', 'backup_get_job', 'backup_save_job', 'backup_delete_job', 'backup_test_destination', 'backup_run', 'backup_execute',
  'backup_cancel', 'backup_status', 'backup_pause', 'backup_resume', 'backup_list_versions', 'backup_list_files', 'backup_search', 'backup_compare',
  'backup_run_history', 'backup_export_cli', 'backup_capabilities', 'backup_preview_sources', 'backup_effective_job', 'backup_list_destinations',
  'backup_database_inventory', 'backup_diagnose', 'backup_job_placeholders', 'backup_test_secret_providers', 'backup_start_operation',
  'backup_cloud_providers', 'backup_remote_list', 'backup_get_global_defaults', 'backup_set_global_defaults', 'backup_test_notification',
] as const;

export const RPC_ALLOW: ReadonlySet<string> = new Set(RPC_COMMANDS);
