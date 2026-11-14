import { describe, expect, it } from "vitest";
import {
  POPULAR_RCLONE_BACKENDS,
  changedParams,
  describeStatus,
  fieldValue,
  isOAuthQuestion,
  missingRequired,
  optionExamples,
  popularBackends,
  providerMatches,
  visibleOptions,
  type RcloneOption,
  type RcloneProvider,
  type RcloneStatus,
} from "@/lib/rclone";

function opt(name: string, patch: Partial<RcloneOption> = {}): RcloneOption {
  return { name, help: `${name} help`, provider: "", default: "", examples: [], required: false, is_password: false, sensitive: false, advanced: false, exclusive: false, hidden: false, type: "string", ...patch };
}

const S3: RcloneProvider = {
  name: "s3",
  description: "Amazon S3 Compliant Storage Providers",
  prefix: "s3",
  hidden: false,
  options: [
    opt("provider", { default: "", exclusive: true, examples: [{ value: "AWS", help: "Amazon", provider: "" }, { value: "Ceph", help: "Ceph", provider: "" }] }),
    opt("access_key_id"),
    opt("secret_access_key", { sensitive: true }),
    opt("region", { provider: "AWS", examples: [{ value: "us-east-1", help: "", provider: "" }] }),
    opt("region", { provider: "!AWS,Ceph", help: "other region" }),
    opt("endpoint", { provider: "Ceph", required: true }),
    opt("chunk_size", { advanced: true, default: "5Mi" }),
    opt("internal", { hidden: true }),
  ],
};

describe("rclone schema helpers", () => {
  it("matches provider filters", () => {
    expect(providerMatches("", "AWS")).toBe(true);
    expect(providerMatches("AWS,Ceph", "Ceph")).toBe(true);
    expect(providerMatches("AWS,Ceph", "Minio")).toBe(false);
    expect(providerMatches("!AWS,Ceph", "Minio")).toBe(true);
    expect(providerMatches("!AWS,Ceph", "AWS")).toBe(false);
  });

  it("shows options for the chosen provider, advanced ones only in Advanced", () => {
    const aws = visibleOptions(S3, { provider: "AWS" }, false).map((o) => o.name);
    expect(aws).toEqual(["provider", "access_key_id", "secret_access_key", "region"]);
    expect(visibleOptions(S3, { provider: "AWS" }, false).find((o) => o.name === "region")?.provider).toBe("AWS");
    const ceph = visibleOptions(S3, { provider: "Ceph" }, false).map((o) => o.name);
    expect(ceph).toContain("endpoint");
    const minio = visibleOptions(S3, { provider: "Minio" }, false);
    expect(minio.find((o) => o.name === "region")?.help).toBe("other region");
    expect(visibleOptions(S3, { provider: "AWS" }, true).map((o) => o.name)).toContain("chunk_size");
    expect(visibleOptions(S3, {}, true).map((o) => o.name)).not.toContain("internal");
    expect(optionExamples(S3.options[3], { provider: "AWS" })).toHaveLength(1);
  });

  it("sends only changed values and never a blank secret", () => {
    const options = visibleOptions(S3, { provider: "AWS" }, true);
    expect(changedParams(options, { provider: "AWS", access_key_id: "AK", secret_access_key: "", chunk_size: "5Mi" })).toEqual({ provider: "AWS", access_key_id: "AK" });
    // Update: compared with what is stored.
    expect(changedParams(options, { provider: "AWS", access_key_id: "AK", secret_access_key: "new" }, { provider: "AWS", access_key_id: "AK" })).toEqual({ secret_access_key: "new" });
    expect(fieldValue(opt("x", { default: "d" }), {})).toBe("d");
    expect(fieldValue(opt("p", { is_password: true, default: "d" }), {})).toBe("");
  });

  // Regression (WS-Z2): clearing a field that has a default sent `key=`,
  // which rclone stores as an empty setting in place of the default.
  it("a cleared field means the default and is not written as an empty setting", () => {
    const options = visibleOptions(S3, { provider: "AWS" }, true);
    // Create: chunk_size cleared, access_key_id left blank.
    expect(changedParams(options, { provider: "AWS", access_key_id: "", chunk_size: "" })).toEqual({ provider: "AWS" });
    // Update: a stored chunk size cleared goes back to the default.
    expect(changedParams(options, { provider: "AWS", chunk_size: "" }, { provider: "AWS", chunk_size: "64Mi" })).toEqual({ chunk_size: "5Mi" });
    // Nothing stored and nothing typed: nothing sent.
    expect(changedParams(options, { provider: "AWS", region: "" }, { provider: "AWS" })).toEqual({});
    // A cleared required field with a default is filled by it.
    expect(missingRequired([opt("bucket", { required: true, default: "main" })], { bucket: "" })).toEqual([]);
  });

  it("reports missing required values (stored secrets count)", () => {
    const options = [opt("remote", { required: true }), opt("password", { required: true, is_password: true })];
    expect(missingRequired(options, {}).map((o) => o.name)).toEqual(["remote", "password"]);
    expect(missingRequired(options, { remote: "/x" }, ["password"])).toEqual([]);
  });

  it("maps the Popular list onto the installed providers", () => {
    const providers: RcloneProvider[] = [
      { ...S3, name: "box", prefix: "box", options: [] },
      { ...S3, name: "google photos", prefix: "gphotos", options: [] },
    ];
    const p = popularBackends(providers);
    expect(p).toHaveLength(POPULAR_RCLONE_BACKENDS.length);
    expect(p.find((b) => b.label === "Box")?.provider?.name).toBe("box");
    expect(p.find((b) => b.label === "Google Photos")?.provider?.name).toBe("google photos");
    expect(p.find((b) => b.label === "MEGA")?.provider).toBeNull();
    for (const label of ["Box", "pCloud", "MEGA", "Proton Drive", "iCloud Drive", "Google Photos", "Yandex Disk", "HiDrive", "Koofr", "Oracle Object Storage", "Gofile", "HTTP", "put.io", "Zoho WorkDrive", "Citrix ShareFile"]) {
      expect(POPULAR_RCLONE_BACKENDS.some((b) => b.label === label)).toBe(true);
    }
  });

  it("recognises the OAuth question and describes the engine", () => {
    expect(isOAuthQuestion({ state: "*oauth-islocal,teamdrive,,", option: opt("config_is_local"), error: "", result: "", done: false })).toBe(true);
    expect(isOAuthQuestion({ state: "x", option: opt("region"), error: "", result: "", done: false })).toBe(false);
    const base: RcloneStatus = { path: "/usr/bin/rclone", source: "path", found: true, version: "v1.69.1", meets_minimum: true, min_version: "1.56.0", config_path: null, managed: false, error: null };
    expect(describeStatus(base)).toBe("v1.69.1 (/usr/bin/rclone)");
    expect(describeStatus({ ...base, version: null, found: false, source: "none", error: "rclone is not installed" })).toBe("not installed");
  });
});
