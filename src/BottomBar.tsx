import { openUrl } from "@tauri-apps/plugin-opener";

export type TabId = "reddit" | "account" | "info" | "github" | "options";

interface BottomBarProps {
  activeTab: TabId;
  onTabChange: (tab: TabId) => void;
}

const TABS: { id: TabId; label: string; url?: string }[] = [
  { id: "reddit", label: "REDDIT", url: "https://www.reddit.com/r/TheSecretWorld/" },
  { id: "account", label: "ACCOUNT", url: "https://register.thesecretworld.com/account/" },
  { id: "info", label: "INFO" },
  { id: "github", label: "GITHUB", url: "https://github.com/EndlessVex/TSW-Modern" },
  { id: "options", label: "OPTIONS" },
];

export default function BottomBar({ activeTab, onTabChange }: BottomBarProps) {
  function handleClick(tab: typeof TABS[number]) {
    if (tab.url) {
      openUrl(tab.url).catch(() => {});
      return;
    }
    onTabChange(tab.id);
  }

  return (
    <nav className="bottom-bar">
      {TABS.map((tab) => (
        <button
          key={tab.id}
          className={`bottom-bar-tab${activeTab === tab.id ? " bottom-bar-tab-active" : ""}`}
          onClick={() => handleClick(tab)}
        >
          {tab.label}
        </button>
      ))}
    </nav>
  );
}
