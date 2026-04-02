import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { store } from "./store";

/** Matches the Rust AuthResult struct from lib.rs */
interface AuthResult {
  success: boolean;
  message: string;
}

interface LoginFormProps {
  installValid: boolean;
  installPath: string | null;
  dxVersion: string;
  launching: boolean;
  patching: boolean;
  verifying: boolean;
  repairing: boolean;
  onLaunchStart: () => void;
  onLaunchEnd: () => void;
}

export default function LoginForm({
  installValid,
  installPath,
  dxVersion,
  launching,
  patching,
  verifying,
  repairing,
  onLaunchStart,
  onLaunchEnd,
}: LoginFormProps) {
  const [username, setUsername] = useState<string>("");
  const [password, setPassword] = useState<string>("");
  const [rememberMe, setRememberMe] = useState(false);
  const [loginState, setLoginState] = useState<"idle" | "authenticating" | "authenticated" | "error">("idle");
  const [loginMessage, setLoginMessage] = useState<string | null>(null);

  // Load saved credentials on mount
  useEffect(() => {
    async function loadCredentials() {
      try {
        const savedRemember = await store.get<boolean>("remember_me");
        if (savedRemember) {
          setRememberMe(true);
          const savedUsername = await store.get<string>("saved_username");
          if (savedUsername) setUsername(savedUsername);
        }
      } catch (err) {
        console.error("Failed to load saved credentials:", err);
      }
    }
    loadCredentials();
  }, []);

  async function handleLoginAndLaunch() {
    if (!installPath || !installValid || launching || patching) return;

    setLoginState("authenticating");
    setLoginMessage(null);

    try {
      const result = await invoke<AuthResult>("authenticate", { username, password });

      if (!result.success) {
        setLoginState("error");
        setLoginMessage(result.message);
        return;
      }

      // Save username if remember-me is checked
      if (rememberMe) {
        await store.set("saved_username", username);
        await store.set("remember_me", true);
        await store.save();
      } else {
        await store.delete("saved_username");
        await store.set("remember_me", false);
        await store.save();
      }

      setLoginState("authenticated");
      setLoginMessage(result.message);

      // Launch the game
      onLaunchStart();
      try {
        await invoke("launch_game", { installPath, dxVersion });
      } finally {
        setTimeout(() => onLaunchEnd(), 2000);
      }
    } catch (err) {
      setLoginState("error");
      setLoginMessage(String(err));
    }
  }

  const busy = launching || patching || verifying || repairing || loginState === "authenticating";

  return (
    <section className="section">
      <label className="section-label">Account Login</label>
      <div className="login-form">
        <input
          type="text"
          className="login-input"
          placeholder="Username"
          value={username}
          onChange={(e) => setUsername(e.target.value)}
          disabled={loginState === "authenticating"}
        />
        <input
          type="password"
          className="login-input"
          placeholder="Password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          disabled={loginState === "authenticating"}
        />
        <label className="remember-me">
          <input
            type="checkbox"
            checked={rememberMe}
            onChange={(e) => setRememberMe(e.target.checked)}
          />
          Remember username
        </label>
        {loginMessage && (
          <div className={`login-message ${loginState === "error" ? "login-message-error" : "login-message-ok"}`}>
            {loginMessage}
          </div>
        )}
        <button
          className="btn btn-login"
          disabled={!installValid || busy}
          onClick={handleLoginAndLaunch}
        >
          {loginState === "authenticating" ? "Authenticating…" : "Login & Play"}
        </button>
      </div>
      <p className="login-hint">
        Or use the Launch button below to start the game and log in there.
      </p>
    </section>
  );
}
