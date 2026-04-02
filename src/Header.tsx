interface HeaderProps {
  currentView: "main" | "settings";
  onToggleSettings: () => void;
}

export default function Header({ currentView, onToggleSettings }: HeaderProps) {
  return (
    <header className="app-header">
      <h1 className="app-title">Secret World Launcher</h1>
      <button
        className={`header-settings-btn${currentView === "settings" ? " header-settings-active" : ""}`}
        onClick={onToggleSettings}
        title={currentView === "settings" ? "Back to launcher" : "Settings"}
        aria-label={currentView === "settings" ? "Back to launcher" : "Settings"}
      >
        ⚙
      </button>
    </header>
  );
}
