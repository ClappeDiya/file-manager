/**
 * S3 Advanced Object Controls Panel
 *
 * Features:
 * - View/edit object ACL (private, public-read, etc.)
 * - View/change storage class (STANDARD, GLACIER, etc.)
 * - View encryption info (SSE-S3, SSE-KMS)
 * - View bucket settings (versioning, default encryption, acceleration)
 * - Toggle transfer acceleration
 *
 * Advanced mode only (progressive disclosure):
 * - Lifecycle Rules: view/add/delete bucket lifecycle rules
 * - CloudFront: list distributions, create cache invalidations
 * - File Versions: browse object versions in versioned buckets, restore old versions
 * - MFA Delete: status indicator for MFA Delete on the bucket
 *
 * Shows contextually when protocol is S3/S3-compatible.
 */
import { useState, useCallback, useEffect } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { formatBytes } from "@/lib/format-bytes";
import { useUIStore } from "@/stores/ui-store";

// ── Types (mirrors Rust types) ──

interface S3ObjectProperties {
  key: string;
  acl: string;
  storage_class: string;
  encryption: string;
  encryption_key_id: string | null;
  content_type: string | null;
  content_length: number;
  last_modified: string | null;
  etag: string | null;
  version_id: string | null;
}

interface S3BucketSettings {
  default_encryption: string;
  default_encryption_key_id: string | null;
  versioning_enabled: boolean;
  versioning_status: string;
  acceleration_enabled: boolean;
}

interface LifecycleRule {
  id: string;
  prefix: string;
  transitions: string;
  expiration: string;
  status: string;
}

interface NewLifecycleRule {
  id: string;
  prefix: string;
  transition_days: number | null;
  transition_storage_class: string;
  expiration_days: number | null;
}

interface CloudFrontDistribution {
  id: string;
  domain: string;
  status: string;
  enabled: boolean;
}

interface FileVersion {
  version_id: string;
  last_modified: string;
  size: number;
  is_latest: boolean;
}

interface MfaDeleteStatus {
  enabled: boolean;
}

// ── Constants ──

const ACL_OPTIONS = [
  { value: "private", label: "Private" },
  { value: "public-read", label: "Public Read" },
  { value: "public-read-write", label: "Public Read/Write" },
  { value: "authenticated-read", label: "Authenticated Read" },
  { value: "bucket-owner-read", label: "Bucket Owner Read" },
  { value: "bucket-owner-full-control", label: "Bucket Owner Full Control" },
];

const STORAGE_CLASS_OPTIONS = [
  { value: "STANDARD", label: "Standard" },
  { value: "REDUCED_REDUNDANCY", label: "Reduced Redundancy" },
  { value: "STANDARD_IA", label: "Standard-IA (Infrequent)" },
  { value: "ONEZONE_IA", label: "One Zone-IA" },
  { value: "INTELLIGENT_TIERING", label: "Intelligent-Tiering" },
  { value: "GLACIER", label: "Glacier Flexible" },
  { value: "GLACIER_IR", label: "Glacier Instant Retrieval" },
  { value: "DEEP_ARCHIVE", label: "Glacier Deep Archive" },
];

// ── Helpers ──

function formatDate(dateStr: string | null): string {
  if (!dateStr) return "-";
  try {
    return new Date(dateStr).toLocaleString();
  } catch {
    return dateStr;
  }
}

function encryptionLabel(encryption: string): string {
  switch (encryption) {
    case "AES256":
      return "SSE-S3 (AES-256)";
    case "aws:kms":
      return "SSE-KMS";
    case "None":
      return "None";
    default:
      return encryption;
  }
}

// ── Object Properties Section ──

function ObjectPropertiesSection({
  connectionId,
  objectKey,
  onError,
}: {
  connectionId: string;
  objectKey: string;
  onError: (msg: string) => void;
}) {
  const [properties, setProperties] = useState<S3ObjectProperties | null>(null);
  const [loading, setLoading] = useState(true);
  const [pendingAcl, setPendingAcl] = useState<string | null>(null);
  const [pendingStorageClass, setPendingStorageClass] = useState<string | null>(
    null,
  );
  const [applying, setApplying] = useState(false);

  const fetchProperties = useCallback(async () => {
    setLoading(true);
    try {
      const props = await tauriInvoke<S3ObjectProperties>(
        "s3_get_object_properties",
        { connectionId, key: objectKey },
        {
          key: objectKey,
          acl: "private",
          storage_class: "STANDARD",
          encryption: "None",
          encryption_key_id: null,
          content_type: "application/octet-stream",
          content_length: 0,
          last_modified: null,
          etag: null,
          version_id: null,
        },
      );
      setProperties(props);
      setPendingAcl(props.acl);
      setPendingStorageClass(props.storage_class);
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  }, [connectionId, objectKey, onError]);

  useEffect(() => {
    fetchProperties();
  }, [fetchProperties]);

  const hasChanges =
    properties &&
    (pendingAcl !== properties.acl ||
      pendingStorageClass !== properties.storage_class);

  const applyChanges = async () => {
    if (!properties) return;
    setApplying(true);
    onError("");
    try {
      if (pendingAcl && pendingAcl !== properties.acl) {
        await tauriInvoke("s3_set_object_acl", {
          connectionId,
          key: objectKey,
          acl: pendingAcl,
        });
      }
      if (
        pendingStorageClass &&
        pendingStorageClass !== properties.storage_class
      ) {
        await tauriInvoke("s3_set_storage_class", {
          connectionId,
          key: objectKey,
          storageClass: pendingStorageClass,
        });
      }
      await fetchProperties();
    } catch (err) {
      onError(String(err));
    } finally {
      setApplying(false);
    }
  };

  if (loading) {
    return (
      <div className="p-3 text-sm text-gray-500 dark:text-gray-400">
        Loading object properties...
      </div>
    );
  }

  if (!properties) {
    return (
      <div className="p-3 text-sm text-gray-500 dark:text-gray-400">
        No properties available.
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <h4 className="text-xs font-semibold uppercase text-gray-500 dark:text-gray-400 tracking-wide">
        Object Properties
      </h4>

      {/* Read-only info */}
      <div className="grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
        <span className="text-gray-500 dark:text-gray-400">Key</span>
        <span className="truncate font-mono" title={properties.key}>
          {properties.key}
        </span>
        <span className="text-gray-500 dark:text-gray-400">Size</span>
        <span>{formatBytes(properties.content_length, { decimals: 2 })}</span>
        <span className="text-gray-500 dark:text-gray-400">Content Type</span>
        <span>{properties.content_type || "-"}</span>
        <span className="text-gray-500 dark:text-gray-400">Last Modified</span>
        <span>{formatDate(properties.last_modified)}</span>
        <span className="text-gray-500 dark:text-gray-400">ETag</span>
        <span className="truncate font-mono" title={properties.etag || ""}>
          {properties.etag || "-"}
        </span>
        {properties.version_id && (
          <>
            <span className="text-gray-500 dark:text-gray-400">
              Version ID
            </span>
            <span className="truncate font-mono">{properties.version_id}</span>
          </>
        )}
        <span className="text-gray-500 dark:text-gray-400">Encryption</span>
        <span>
          {encryptionLabel(properties.encryption)}
          {properties.encryption_key_id && (
            <span
              className="block truncate text-gray-400 font-mono"
              title={properties.encryption_key_id}
            >
              Key: {properties.encryption_key_id}
            </span>
          )}
        </span>
      </div>

      {/* Editable fields */}
      <div className="space-y-2 pt-2 border-t border-gray-200 dark:border-gray-700">
        <div>
          <label className="block text-xs font-medium mb-1">
            Access Control (ACL)
          </label>
          <select
            value={pendingAcl || "private"}
            onChange={(e) => setPendingAcl(e.target.value)}
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          >
            {ACL_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
        </div>

        <div>
          <label className="block text-xs font-medium mb-1">
            Storage Class
          </label>
          <select
            value={pendingStorageClass || "STANDARD"}
            onChange={(e) => setPendingStorageClass(e.target.value)}
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          >
            {STORAGE_CLASS_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
          {pendingStorageClass !== properties.storage_class && (
            <p className="text-xs text-amber-600 dark:text-amber-400 mt-1">
              Changing storage class creates a copy-in-place operation.
            </p>
          )}
        </div>
      </div>

      {/* Apply button */}
      {hasChanges && (
        <div className="flex justify-end pt-1">
          <button
            onClick={applyChanges}
            disabled={applying}
            className="text-sm px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
          >
            {applying ? "Applying..." : "Apply Changes"}
          </button>
        </div>
      )}
    </div>
  );
}

// ── Bucket Settings Section ──

function BucketSettingsSection({
  connectionId,
  onError,
}: {
  connectionId: string;
  onError: (msg: string) => void;
}) {
  const [settings, setSettings] = useState<S3BucketSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [togglingAcceleration, setTogglingAcceleration] = useState(false);

  const fetchSettings = useCallback(async () => {
    setLoading(true);
    try {
      const s = await tauriInvoke<S3BucketSettings>(
        "s3_get_bucket_settings",
        { connectionId },
        {
          default_encryption: "None",
          default_encryption_key_id: null,
          versioning_enabled: false,
          versioning_status: "Disabled",
          acceleration_enabled: false,
        },
      );
      setSettings(s);
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  }, [connectionId, onError]);

  useEffect(() => {
    fetchSettings();
  }, [fetchSettings]);

  const toggleAcceleration = async () => {
    if (!settings) return;
    setTogglingAcceleration(true);
    onError("");
    try {
      await tauriInvoke("s3_set_transfer_acceleration", {
        connectionId,
        enabled: !settings.acceleration_enabled,
      });
      await fetchSettings();
    } catch (err) {
      onError(String(err));
    } finally {
      setTogglingAcceleration(false);
    }
  };

  if (loading) {
    return (
      <div className="p-3 text-sm text-gray-500 dark:text-gray-400">
        Loading bucket settings...
      </div>
    );
  }

  if (!settings) {
    return (
      <div className="p-3 text-sm text-gray-500 dark:text-gray-400">
        No bucket settings available.
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <h4 className="text-xs font-semibold uppercase text-gray-500 dark:text-gray-400 tracking-wide">
        Bucket Settings
      </h4>

      <div className="grid grid-cols-2 gap-x-3 gap-y-1.5 text-xs">
        <span className="text-gray-500 dark:text-gray-400">
          Default Encryption
        </span>
        <span>
          {encryptionLabel(settings.default_encryption)}
          {settings.default_encryption_key_id && (
            <span
              className="block truncate text-gray-400 font-mono"
              title={settings.default_encryption_key_id}
            >
              Key: {settings.default_encryption_key_id}
            </span>
          )}
        </span>

        <span className="text-gray-500 dark:text-gray-400">Versioning</span>
        <span className="flex items-center gap-1.5">
          <span
            className={`w-2 h-2 rounded-full ${
              settings.versioning_enabled ? "bg-green-500" : "bg-gray-400"
            }`}
          />
          {settings.versioning_status}
        </span>
      </div>

      {/* Transfer Acceleration toggle */}
      <div className="flex items-center justify-between pt-2 border-t border-gray-200 dark:border-gray-700">
        <div>
          <div className="text-xs font-medium">Transfer Acceleration</div>
          <div className="text-xs text-gray-500 dark:text-gray-400">
            Use CloudFront edge locations for faster uploads
          </div>
        </div>
        <button
          onClick={toggleAcceleration}
          disabled={togglingAcceleration}
          className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors ${
            settings.acceleration_enabled
              ? "bg-blue-500"
              : "bg-gray-300 dark:bg-gray-600"
          } disabled:opacity-50`}
          role="switch"
          aria-checked={settings.acceleration_enabled}
          type="button"
        >
          <span
            className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
              settings.acceleration_enabled
                ? "translate-x-4"
                : "translate-x-0.5"
            }`}
          />
        </button>
      </div>
    </div>
  );
}

// ── Collapsible Section (progressive disclosure) ──

function CollapsibleSection({
  title,
  defaultOpen = false,
  children,
}: {
  title: string;
  defaultOpen?: boolean;
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="border border-gray-200 dark:border-gray-700 rounded-lg overflow-hidden">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="w-full flex items-center justify-between px-3 py-2 text-xs font-medium text-left hover:bg-gray-50 dark:hover:bg-gray-800/50 transition-colors"
      >
        <span>{title}</span>
        <span className="text-gray-400 text-xs">
          {open ? "collapse" : "expand"}
        </span>
      </button>
      {open && (
        <div className="px-3 pb-3 border-t border-gray-200 dark:border-gray-700">
          {children}
        </div>
      )}
    </div>
  );
}

// ── Lifecycle Rules Section (advanced only) ──

function LifecycleRulesSection({
  connectionId,
  onError,
}: {
  connectionId: string;
  onError: (msg: string) => void;
}) {
  const [rules, setRules] = useState<LifecycleRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [showForm, setShowForm] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [newRule, setNewRule] = useState<NewLifecycleRule>({
    id: "",
    prefix: "",
    transition_days: null,
    transition_storage_class: "GLACIER",
    expiration_days: null,
  });

  const fetchRules = useCallback(async () => {
    setLoading(true);
    try {
      const result = await tauriInvoke<LifecycleRule[]>(
        "s3_get_lifecycle_rules",
        { connectionId },
        [],
      );
      setRules(result);
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  }, [connectionId, onError]);

  useEffect(() => {
    fetchRules();
  }, [fetchRules]);

  const addRule = async () => {
    if (!newRule.id.trim()) {
      onError("Rule ID is required");
      return;
    }
    setSubmitting(true);
    onError("");
    try {
      await tauriInvoke("s3_put_lifecycle_rule", {
        connectionId,
        ruleId: newRule.id,
        prefix: newRule.prefix,
        transitionDays: newRule.transition_days,
        transitionStorageClass: newRule.transition_storage_class,
        expirationDays: newRule.expiration_days,
      });
      setNewRule({
        id: "",
        prefix: "",
        transition_days: null,
        transition_storage_class: "GLACIER",
        expiration_days: null,
      });
      setShowForm(false);
      await fetchRules();
    } catch (err) {
      onError(String(err));
    } finally {
      setSubmitting(false);
    }
  };

  const deleteRule = async (ruleId: string) => {
    setDeleting(ruleId);
    onError("");
    try {
      await tauriInvoke("s3_delete_lifecycle_rule", {
        connectionId,
        ruleId,
      });
      await fetchRules();
    } catch (err) {
      onError(String(err));
    } finally {
      setDeleting(null);
    }
  };

  return (
    <CollapsibleSection title="Lifecycle Rules">
      {loading ? (
        <div className="py-2 text-xs text-gray-500 dark:text-gray-400">
          Loading lifecycle rules...
        </div>
      ) : (
        <div className="space-y-2 pt-2">
          {rules.length === 0 ? (
            <div className="text-xs text-gray-500 dark:text-gray-400">
              No lifecycle rules configured.
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-xs">
                <thead>
                  <tr className="text-left text-gray-500 dark:text-gray-400 border-b border-gray-200 dark:border-gray-700">
                    <th className="pb-1 pr-2 font-medium">ID</th>
                    <th className="pb-1 pr-2 font-medium">Prefix</th>
                    <th className="pb-1 pr-2 font-medium">Transitions</th>
                    <th className="pb-1 pr-2 font-medium">Expiration</th>
                    <th className="pb-1 pr-2 font-medium">Status</th>
                    <th className="pb-1 font-medium"></th>
                  </tr>
                </thead>
                <tbody>
                  {rules.map((rule) => (
                    <tr
                      key={rule.id}
                      className="border-b border-gray-100 dark:border-gray-800"
                    >
                      <td className="py-1 pr-2 font-mono truncate max-w-[80px]" title={rule.id}>
                        {rule.id}
                      </td>
                      <td className="py-1 pr-2 truncate max-w-[60px]" title={rule.prefix}>
                        {rule.prefix || "*"}
                      </td>
                      <td className="py-1 pr-2">{rule.transitions || "-"}</td>
                      <td className="py-1 pr-2">{rule.expiration || "-"}</td>
                      <td className="py-1 pr-2">
                        <span
                          className={`inline-block px-1.5 py-0.5 rounded text-xs ${
                            rule.status === "Enabled"
                              ? "bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400"
                              : "bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-400"
                          }`}
                        >
                          {rule.status}
                        </span>
                      </td>
                      <td className="py-1 text-right">
                        <button
                          onClick={() => deleteRule(rule.id)}
                          disabled={deleting === rule.id}
                          className="text-red-500 hover:text-red-700 dark:text-red-400 dark:hover:text-red-300 disabled:opacity-50"
                          title="Delete rule"
                        >
                          {deleting === rule.id ? "..." : "Delete"}
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}

          {/* Add rule form */}
          {showForm ? (
            <div className="space-y-2 pt-2 border-t border-gray-200 dark:border-gray-700">
              <div>
                <label className="block text-xs font-medium mb-1">Rule ID</label>
                <input
                  type="text"
                  value={newRule.id}
                  onChange={(e) =>
                    setNewRule((prev) => ({ ...prev, id: e.target.value }))
                  }
                  placeholder="e.g., archive-old-logs"
                  className="w-full text-xs px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                />
              </div>
              <div>
                <label className="block text-xs font-medium mb-1">Prefix Filter</label>
                <input
                  type="text"
                  value={newRule.prefix}
                  onChange={(e) =>
                    setNewRule((prev) => ({ ...prev, prefix: e.target.value }))
                  }
                  placeholder="e.g., logs/ (empty = all objects)"
                  className="w-full text-xs px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                />
              </div>
              <div className="grid grid-cols-2 gap-2">
                <div>
                  <label className="block text-xs font-medium mb-1">
                    Transition Days
                  </label>
                  <input
                    type="number"
                    min="1"
                    value={newRule.transition_days ?? ""}
                    onChange={(e) =>
                      setNewRule((prev) => ({
                        ...prev,
                        transition_days: e.target.value
                          ? parseInt(e.target.value, 10)
                          : null,
                      }))
                    }
                    placeholder="e.g., 90"
                    className="w-full text-xs px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                  />
                </div>
                <div>
                  <label className="block text-xs font-medium mb-1">
                    Storage Class
                  </label>
                  <select
                    value={newRule.transition_storage_class}
                    onChange={(e) =>
                      setNewRule((prev) => ({
                        ...prev,
                        transition_storage_class: e.target.value,
                      }))
                    }
                    className="w-full text-xs px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                  >
                    {STORAGE_CLASS_OPTIONS.filter(
                      (o) => o.value !== "STANDARD",
                    ).map((opt) => (
                      <option key={opt.value} value={opt.value}>
                        {opt.label}
                      </option>
                    ))}
                  </select>
                </div>
              </div>
              <div>
                <label className="block text-xs font-medium mb-1">
                  Expiration Days
                </label>
                <input
                  type="number"
                  min="1"
                  value={newRule.expiration_days ?? ""}
                  onChange={(e) =>
                    setNewRule((prev) => ({
                      ...prev,
                      expiration_days: e.target.value
                        ? parseInt(e.target.value, 10)
                        : null,
                    }))
                  }
                  placeholder="e.g., 365 (empty = no expiration)"
                  className="w-full text-xs px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                />
              </div>
              <div className="flex gap-2 pt-1">
                <button
                  onClick={addRule}
                  disabled={submitting}
                  className="text-xs px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
                >
                  {submitting ? "Adding..." : "Add Rule"}
                </button>
                <button
                  onClick={() => setShowForm(false)}
                  className="text-xs px-3 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-50 dark:hover:bg-gray-800"
                >
                  Cancel
                </button>
              </div>
            </div>
          ) : (
            <button
              onClick={() => setShowForm(true)}
              className="text-xs text-blue-500 hover:text-blue-600 dark:text-blue-400 dark:hover:text-blue-300 pt-1"
            >
              + Add Lifecycle Rule
            </button>
          )}
        </div>
      )}
    </CollapsibleSection>
  );
}

// ── CloudFront Section (advanced only) ──

function CloudFrontSection({
  connectionId,
  onError,
}: {
  connectionId: string;
  onError: (msg: string) => void;
}) {
  const [distributions, setDistributions] = useState<CloudFrontDistribution[]>(
    [],
  );
  const [loading, setLoading] = useState(true);
  const [invalidationDistId, setInvalidationDistId] = useState<string | null>(
    null,
  );
  const [invalidationPaths, setInvalidationPaths] = useState("/*");
  const [creatingInvalidation, setCreatingInvalidation] = useState(false);

  const fetchDistributions = useCallback(async () => {
    setLoading(true);
    try {
      const result = await tauriInvoke<CloudFrontDistribution[]>(
        "s3_list_distributions",
        { connectionId },
        [],
      );
      setDistributions(result);
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  }, [connectionId, onError]);

  useEffect(() => {
    fetchDistributions();
  }, [fetchDistributions]);

  const createInvalidation = async () => {
    if (!invalidationDistId) return;
    const paths = invalidationPaths
      .split("\n")
      .map((p) => p.trim())
      .filter(Boolean);
    if (paths.length === 0) {
      onError("At least one invalidation path is required");
      return;
    }
    setCreatingInvalidation(true);
    onError("");
    try {
      await tauriInvoke("s3_create_invalidation", {
        connectionId,
        distributionId: invalidationDistId,
        paths,
      });
      setInvalidationDistId(null);
      setInvalidationPaths("/*");
    } catch (err) {
      onError(String(err));
    } finally {
      setCreatingInvalidation(false);
    }
  };

  return (
    <CollapsibleSection title="CloudFront Distributions">
      {loading ? (
        <div className="py-2 text-xs text-gray-500 dark:text-gray-400">
          Loading distributions...
        </div>
      ) : (
        <div className="space-y-2 pt-2">
          {distributions.length === 0 ? (
            <div className="text-xs text-gray-500 dark:text-gray-400">
              No CloudFront distributions found.
            </div>
          ) : (
            <div className="space-y-1.5">
              {distributions.map((dist) => (
                <div
                  key={dist.id}
                  className="flex items-center justify-between text-xs p-2 rounded border border-gray-200 dark:border-gray-700"
                >
                  <div className="min-w-0 flex-1">
                    <div className="font-mono truncate" title={dist.domain}>
                      {dist.domain}
                    </div>
                    <div className="text-gray-500 dark:text-gray-400 flex items-center gap-2 mt-0.5">
                      <span>{dist.status}</span>
                      <span
                        className={`inline-block w-1.5 h-1.5 rounded-full ${
                          dist.enabled ? "bg-green-500" : "bg-gray-400"
                        }`}
                        title={dist.enabled ? "Enabled" : "Disabled"}
                      />
                      <span>{dist.enabled ? "Enabled" : "Disabled"}</span>
                    </div>
                  </div>
                  <button
                    onClick={() =>
                      setInvalidationDistId(
                        invalidationDistId === dist.id ? null : dist.id,
                      )
                    }
                    className="ml-2 text-xs text-blue-500 hover:text-blue-600 dark:text-blue-400 dark:hover:text-blue-300 whitespace-nowrap"
                  >
                    {invalidationDistId === dist.id
                      ? "Cancel"
                      : "Invalidate"}
                  </button>
                </div>
              ))}
            </div>
          )}

          {/* Invalidation form */}
          {invalidationDistId && (
            <div className="space-y-2 pt-2 border-t border-gray-200 dark:border-gray-700">
              <label className="block text-xs font-medium">
                Invalidation Paths
              </label>
              <textarea
                value={invalidationPaths}
                onChange={(e) => setInvalidationPaths(e.target.value)}
                placeholder={"/*\n/images/*\n/index.html"}
                rows={3}
                className="w-full text-xs px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono resize-y"
              />
              <p className="text-xs text-gray-500 dark:text-gray-400">
                One path per line. Use /* to invalidate all objects.
              </p>
              <button
                onClick={createInvalidation}
                disabled={creatingInvalidation}
                className="text-xs px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
              >
                {creatingInvalidation
                  ? "Creating Invalidation..."
                  : "Create Invalidation"}
              </button>
            </div>
          )}
        </div>
      )}
    </CollapsibleSection>
  );
}

// ── File Versions Section (advanced only) ──

function FileVersionsSection({
  connectionId,
  objectKey,
  onError,
}: {
  connectionId: string;
  objectKey: string;
  onError: (msg: string) => void;
}) {
  const [versions, setVersions] = useState<FileVersion[]>([]);
  const [loading, setLoading] = useState(true);
  const [restoring, setRestoring] = useState<string | null>(null);

  const fetchVersions = useCallback(async () => {
    setLoading(true);
    try {
      const result = await tauriInvoke<FileVersion[]>(
        "list_file_versions",
        { connectionId, key: objectKey },
        [],
      );
      setVersions(result);
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  }, [connectionId, objectKey, onError]);

  useEffect(() => {
    fetchVersions();
  }, [fetchVersions]);

  const restoreVersion = async (versionId: string) => {
    setRestoring(versionId);
    onError("");
    try {
      await tauriInvoke("restore_file_version", {
        connectionId,
        key: objectKey,
        versionId,
      });
      await fetchVersions();
    } catch (err) {
      onError(String(err));
    } finally {
      setRestoring(null);
    }
  };

  return (
    <CollapsibleSection title="File Versions">
      {loading ? (
        <div className="py-2 text-xs text-gray-500 dark:text-gray-400">
          Loading versions...
        </div>
      ) : (
        <div className="space-y-1.5 pt-2">
          {versions.length === 0 ? (
            <div className="text-xs text-gray-500 dark:text-gray-400">
              No versions found. Versioning may not be enabled on this bucket.
            </div>
          ) : (
            versions.map((version) => (
              <div
                key={version.version_id}
                className="flex items-center justify-between text-xs p-2 rounded border border-gray-200 dark:border-gray-700"
              >
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span
                      className="font-mono truncate max-w-[120px]"
                      title={version.version_id}
                    >
                      {version.version_id}
                    </span>
                    {version.is_latest && (
                      <span className="inline-block px-1.5 py-0.5 rounded bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400 text-xs whitespace-nowrap">
                        Latest
                      </span>
                    )}
                  </div>
                  <div className="text-gray-500 dark:text-gray-400 mt-0.5">
                    {formatDate(version.last_modified)} &middot;{" "}
                    {formatBytes(version.size, { decimals: 2 })}
                  </div>
                </div>
                {!version.is_latest && (
                  <button
                    onClick={() => restoreVersion(version.version_id)}
                    disabled={restoring === version.version_id}
                    className="ml-2 text-xs text-blue-500 hover:text-blue-600 dark:text-blue-400 dark:hover:text-blue-300 disabled:opacity-50 whitespace-nowrap"
                  >
                    {restoring === version.version_id
                      ? "Restoring..."
                      : "Restore"}
                  </button>
                )}
              </div>
            ))
          )}
        </div>
      )}
    </CollapsibleSection>
  );
}

// ── S3 Object Tags (advanced only) ──

interface S3ObjectTag {
  key: string;
  value: string;
}

function ObjectTagsSection({
  connectionId,
  objectKey,
  onError,
}: {
  connectionId: string;
  objectKey: string;
  onError: (msg: string) => void;
}) {
  const [tags, setTags] = useState<S3ObjectTag[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [newKey, setNewKey] = useState("");
  const [newValue, setNewValue] = useState("");
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [editValue, setEditValue] = useState("");

  const fetchTags = useCallback(async () => {
    setLoading(true);
    try {
      const result = await tauriInvoke<S3ObjectTag[]>(
        "s3_get_object_tags",
        { connectionId, key: objectKey },
        [],
      );
      setTags(result);
    } catch (err) {
      onError(String(err));
    } finally {
      setLoading(false);
    }
  }, [connectionId, objectKey, onError]);

  useEffect(() => {
    fetchTags();
  }, [fetchTags]);

  const saveTags = async (updatedTags: S3ObjectTag[]) => {
    setSaving(true);
    onError("");
    try {
      await tauriInvoke("s3_put_object_tags", {
        connectionId,
        key: objectKey,
        tags: updatedTags,
      });
      setTags(updatedTags);
    } catch (err) {
      onError(String(err));
    } finally {
      setSaving(false);
    }
  };

  const addTag = async () => {
    const trimmedKey = newKey.trim();
    const trimmedValue = newValue.trim();
    if (!trimmedKey) {
      onError("Tag key is required");
      return;
    }
    if (tags.length >= 10) {
      onError("S3 supports a maximum of 10 tags per object");
      return;
    }
    if (tags.some((t) => t.key === trimmedKey)) {
      onError(`Tag key "${trimmedKey}" already exists`);
      return;
    }
    const updated = [...tags, { key: trimmedKey, value: trimmedValue }];
    await saveTags(updated);
    setNewKey("");
    setNewValue("");
  };

  const removeTag = async (index: number) => {
    const updated = tags.filter((_, i) => i !== index);
    await saveTags(updated);
  };

  const startEditValue = (index: number) => {
    setEditingIndex(index);
    setEditValue(tags[index].value);
  };

  const saveEditValue = async () => {
    if (editingIndex === null) return;
    const updated = tags.map((t, i) =>
      i === editingIndex ? { ...t, value: editValue } : t,
    );
    await saveTags(updated);
    setEditingIndex(null);
  };

  const deleteAllTags = async () => {
    setSaving(true);
    onError("");
    try {
      await tauriInvoke("s3_delete_object_tags", {
        connectionId,
        key: objectKey,
      });
      setTags([]);
    } catch (err) {
      onError(String(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <CollapsibleSection title="Object Tags">
      {loading ? (
        <div className="py-2 text-xs text-gray-500 dark:text-gray-400">
          Loading tags...
        </div>
      ) : (
        <div className="space-y-2 pt-2">
          {tags.length === 0 ? (
            <div className="text-xs text-gray-500 dark:text-gray-400">
              No tags on this object.
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-xs">
                <thead>
                  <tr className="text-left text-gray-500 dark:text-gray-400 border-b border-gray-200 dark:border-gray-700">
                    <th className="pb-1 pr-2 font-medium">Key</th>
                    <th className="pb-1 pr-2 font-medium">Value</th>
                    <th className="pb-1 font-medium w-12"></th>
                  </tr>
                </thead>
                <tbody>
                  {tags.map((tag, index) => (
                    <tr
                      key={tag.key}
                      className="border-b border-gray-100 dark:border-gray-800"
                    >
                      <td className="py-1 pr-2 font-mono truncate max-w-[100px]" title={tag.key}>
                        {tag.key}
                      </td>
                      <td className="py-1 pr-2">
                        {editingIndex === index ? (
                          <input
                            type="text"
                            value={editValue}
                            onChange={(e) => setEditValue(e.target.value)}
                            onKeyDown={(e) => {
                              if (e.key === "Enter") saveEditValue();
                              if (e.key === "Escape") setEditingIndex(null);
                            }}
                            className="w-full text-xs px-1 py-0.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono"
                            autoFocus
                          />
                        ) : (
                          <span
                            className="cursor-pointer hover:text-blue-500 dark:hover:text-blue-400 font-mono truncate block max-w-[100px]"
                            title={`${tag.value} (click to edit)`}
                            onClick={() => startEditValue(index)}
                          >
                            {tag.value || <span className="text-gray-400 italic">empty</span>}
                          </span>
                        )}
                      </td>
                      <td className="py-1 text-right">
                        {editingIndex === index ? (
                          <button
                            onClick={saveEditValue}
                            className="text-blue-500 hover:text-blue-700 dark:text-blue-400"
                          >
                            Save
                          </button>
                        ) : (
                          <button
                            onClick={() => removeTag(index)}
                            disabled={saving}
                            className="text-red-500 hover:text-red-700 dark:text-red-400 dark:hover:text-red-300 disabled:opacity-50"
                            title="Remove tag"
                          >
                            Del
                          </button>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {tags.length > 0 && (
                <div className="flex justify-between items-center mt-1">
                  <span className="text-xs text-gray-400">
                    {tags.length}/10 tags
                  </span>
                  <button
                    onClick={deleteAllTags}
                    disabled={saving}
                    className="text-xs text-red-500 hover:text-red-700 dark:text-red-400 disabled:opacity-50"
                  >
                    Remove All
                  </button>
                </div>
              )}
            </div>
          )}

          {/* Add tag row */}
          {tags.length < 10 && (
            <div className="flex gap-1 items-end pt-1">
              <div className="flex-1">
                <input
                  type="text"
                  value={newKey}
                  onChange={(e) => setNewKey(e.target.value)}
                  placeholder="Key"
                  className="w-full text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono"
                />
              </div>
              <div className="flex-1">
                <input
                  type="text"
                  value={newValue}
                  onChange={(e) => setNewValue(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") addTag();
                  }}
                  placeholder="Value"
                  className="w-full text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono"
                />
              </div>
              <button
                onClick={addTag}
                disabled={saving || !newKey.trim()}
                className="text-xs px-2 py-1 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50 whitespace-nowrap"
              >
                {saving ? "..." : "+ Add"}
              </button>
            </div>
          )}
        </div>
      )}
    </CollapsibleSection>
  );
}

// ── MFA Delete Status (advanced only) ──

function MfaDeleteIndicator({
  connectionId,
  onError,
}: {
  connectionId: string;
  onError: (msg: string) => void;
}) {
  const [mfaStatus, setMfaStatus] = useState<MfaDeleteStatus | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const result = await tauriInvoke<MfaDeleteStatus>(
          "s3_get_mfa_delete_status",
          { connectionId },
          { enabled: false },
        );
        if (!cancelled) setMfaStatus(result);
      } catch (err) {
        if (!cancelled) onError(String(err));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [connectionId, onError]);

  if (loading) {
    return null;
  }

  if (!mfaStatus) {
    return null;
  }

  return (
    <div className="flex items-center gap-2 text-xs px-3 py-2 rounded border border-gray-200 dark:border-gray-700">
      <span
        className={`inline-block w-2 h-2 rounded-full ${
          mfaStatus.enabled ? "bg-green-500" : "bg-gray-400"
        }`}
      />
      <span className="text-gray-600 dark:text-gray-300">MFA Delete</span>
      <span
        className={`font-medium ${
          mfaStatus.enabled
            ? "text-green-600 dark:text-green-400"
            : "text-gray-500 dark:text-gray-400"
        }`}
      >
        {mfaStatus.enabled ? "Enabled" : "Disabled"}
      </span>
    </div>
  );
}

// ── Main Panel ──

interface S3PropertiesPanelProps {
  /** The S3 connection/bucket identifier */
  connectionId: string;
  /** The object key to show properties for (null for bucket-only view) */
  objectKey?: string | null;
  /** Whether the current connection is S3-type */
  isS3: boolean;
}

export default function S3PropertiesPanel({
  connectionId,
  objectKey,
  isS3,
}: S3PropertiesPanelProps) {
  const [error, setError] = useState("");
  const appMode = useUIStore((s) => s.appMode);
  const isAdvanced = appMode === "advanced";

  // Only render for S3 connections (progressive disclosure)
  if (!isS3) {
    return null;
  }

  return (
    <div className="border border-gray-200 dark:border-gray-700 rounded-lg overflow-hidden">
      <div className="px-3 py-2 bg-gray-50 dark:bg-gray-800/50 border-b border-gray-200 dark:border-gray-700">
        <h3 className="text-sm font-medium">S3 Properties</h3>
      </div>

      <div className="p-3 space-y-4">
        {error && (
          <div className="text-xs text-red-500 dark:text-red-400 bg-red-50 dark:bg-red-900/20 rounded px-2 py-1.5">
            {error}
          </div>
        )}

        {/* Object Properties - only when a specific object is selected */}
        {objectKey && (
          <ObjectPropertiesSection
            connectionId={connectionId}
            objectKey={objectKey}
            onError={setError}
          />
        )}

        {/* Bucket Settings - always shown for S3 connections */}
        <BucketSettingsSection
          connectionId={connectionId}
          onError={setError}
        />

        {/* Advanced-only sections (progressive disclosure) */}
        {isAdvanced && (
          <>
            {/* MFA Delete Status indicator */}
            <MfaDeleteIndicator
              connectionId={connectionId}
              onError={setError}
            />

            {/* Object Tags - when viewing a specific object */}
            {objectKey && (
              <ObjectTagsSection
                connectionId={connectionId}
                objectKey={objectKey}
                onError={setError}
              />
            )}

            {/* File Versions - only when viewing a specific object */}
            {objectKey && (
              <FileVersionsSection
                connectionId={connectionId}
                objectKey={objectKey}
                onError={setError}
              />
            )}

            {/* Lifecycle Rules - bucket-level management */}
            <LifecycleRulesSection
              connectionId={connectionId}
              onError={setError}
            />

            {/* CloudFront Distributions */}
            <CloudFrontSection
              connectionId={connectionId}
              onError={setError}
            />
          </>
        )}
      </div>
    </div>
  );
}
