/**
 * Password field with a show/hide (eye) toggle. Takes the same props as an
 * `<input>`; `type` is managed by the toggle.
 */
import { useState, type InputHTMLAttributes } from "react";
import { Eye, EyeOff } from "lucide-react";

export function PasswordInput({ className, ...props }: Omit<InputHTMLAttributes<HTMLInputElement>, "type">) {
  const [shown, setShown] = useState(false);
  const label = shown ? "Hide password" : "Show password";
  return (
    <div className="relative">
      <input {...props} type={shown ? "text" : "password"} className={`${className ?? ""} pr-8`} />
      <button
        type="button"
        onClick={() => setShown((v) => !v)}
        className="absolute right-1.5 top-1/2 -translate-y-1/2 p-0.5 text-gray-400 hover:text-gray-600 dark:hover:text-gray-200"
        aria-label={label}
        aria-pressed={shown}
        title={label}
      >
        {shown ? <EyeOff className="h-3.5 w-3.5" aria-hidden="true" /> : <Eye className="h-3.5 w-3.5" aria-hidden="true" />}
      </button>
    </div>
  );
}
