import { type BackupNotification, type NotificationChannel } from "@/stores/backup-store";


export type SimpleType = Exclude<NotificationChannel["type"], "webhook">;


export function newNotification(type: SimpleType): BackupNotification {
  const base = { send_on: ["warning", "error"], credential_id: crypto.randomUUID(), template: "", cert: { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [] } };
  if (type === "telegram") return { type, chat_id: "", api_base: "", ...base };
  if (type === "xmpp") return { type, jid: "", to: "", host: "", port: null, tls: "start_tls", ...base };
  if (type === "slack" || type === "teams") return { type, ...base };
  // New mail channels send HTML (with a text part); older ones keep text only.
  return { type: "email", host: "", port: null, tls: "start_tls", username: "", from: "", to: [], subject: "UFOP {{operation}} {{result}}: {{name}}", html: true, html_template: "", ...base };
}
