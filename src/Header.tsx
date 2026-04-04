import { getCurrentWindow } from "@tauri-apps/api/window";

export default function Header() {
  const appWindow = getCurrentWindow();

  return (
    <header className="app-header">
      <div className="titlebar-buttons">
        <button
          className="titlebar-btn"
          onClick={() => appWindow.minimize()}
          aria-label="Minimize"
          title="Minimize"
        >
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
            <circle cx="7" cy="7" r="6.5" stroke="currentColor" strokeWidth="1" />
            <line x1="4" y1="7" x2="10" y2="7" stroke="currentColor" strokeWidth="1.2" />
          </svg>
        </button>
        <button
          className="titlebar-btn"
          onClick={() => appWindow.close()}
          aria-label="Close"
          title="Close"
        >
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
            <circle cx="7" cy="7" r="6.5" stroke="currentColor" strokeWidth="1" />
            <line x1="4.5" y1="4.5" x2="9.5" y2="9.5" stroke="currentColor" strokeWidth="1.2" />
            <line x1="9.5" y1="4.5" x2="4.5" y2="9.5" stroke="currentColor" strokeWidth="1.2" />
          </svg>
        </button>
      </div>
    </header>
  );
}
