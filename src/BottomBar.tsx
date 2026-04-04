export type TabId = "item-store" | "account" | "info" | "notes" | "options";

interface BottomBarProps {
  activeTab: TabId;
  onTabChange: (tab: TabId) => void;
}

const TABS: { id: TabId; label: string }[] = [
  { id: "item-store", label: "ITEM STORE" },
  { id: "account", label: "ACCOUNT" },
  { id: "info", label: "INFO" },
  { id: "notes", label: "NOTES" },
  { id: "options", label: "OPTIONS" },
];

export default function BottomBar({ activeTab, onTabChange }: BottomBarProps) {
  function handleClick(tab: TabId) {
    if (tab === "item-store") {
      // Open external item store URL instead of switching tabs
      window.open("https://www.thesecretworld.com/#!/item-store", "_blank");
      return;
    }
    onTabChange(tab);
  }

  return (
    <nav className="bottom-bar">
      {TABS.map((tab) => (
        <button
          key={tab.id}
          className={`bottom-bar-tab${activeTab === tab.id ? " bottom-bar-tab-active" : ""}`}
          onClick={() => handleClick(tab.id)}
        >
          {tab.label}
        </button>
      ))}
    </nav>
  );
}
