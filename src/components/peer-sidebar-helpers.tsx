import { Shield, ShieldCheck, ShieldX, ShieldQuestion } from "lucide-react";

export type TrustLevel = "unknown" | "untrusted" | "trusted" | "blocked";


/** Get the appropriate trust icon for a peer's trust level */
export function getTrustIcon(trust: TrustLevel) {
  switch (trust) {
    case "trusted":
      return ShieldCheck;
    case "blocked":
      return ShieldX;
    case "untrusted":
      return ShieldQuestion;
    default:
      return Shield;
  }
}
