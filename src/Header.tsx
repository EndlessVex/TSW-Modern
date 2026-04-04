import { getCurrentWindow } from "@tauri-apps/api/window";

export default function Header() {
  const appWindow = getCurrentWindow();

  return (
    <header className="app-header">
      <div className="titlebar-buttons">
        <button
          className="titlebar-btn titlebar-minimize"
          onClick={() => appWindow.minimize()}
          aria-label="Minimize"
          title="Minimize"
        >
          <svg width="18" height="18" viewBox="0 0 44 44">
            <circle cx="22" cy="22" r="21" fill="#727375" />
            <rect x="8" y="19" width="28" height="5" rx="1" fill="#fefefe" />
          </svg>
        </button>
        <button
          className="titlebar-btn titlebar-close"
          onClick={() => appWindow.close()}
          aria-label="Close"
          title="Close"
        >
          <svg width="18" height="18" viewBox="0 0 44 44">
            <circle cx="22" cy="22" r="21" fill="#727375" />
            <path d="M13 13 L31 31 M31 13 L13 31" stroke="#fefefe" strokeWidth="4" strokeLinecap="round" />
          </svg>
        </button>
      </div>
    </header>
  );
}
