import { cn } from "cn";
import { useState, type ReactElement, type ReactNode } from "react";
import { Tooltip } from "@/components/ui/tooltip";
import type { NpcapStatus } from "../lib/npcap";
import { usePicker } from "../lib/picker";
import { useSettings } from "../lib/SettingsContext";
import { HistoryView } from "./HistoryView";
import { CollapseIcon, HistoryIcon, LiveIcon, ServersIcon, SettingsIcon } from "./icons";
import { MonitorScreen } from "./MonitorScreen";
import { InlineError } from "./Page";
import { ServersView } from "./ServersView";
import { SettingsView } from "./SettingsView";

type Tab = "live" | "history" | "servers" | "settings";

const COLLAPSED_KEY = "efsp.sidebarCollapsed";

/** A per-window convenience; fine if storage is unavailable. */
function readCollapsed(): boolean {
  try {
    return localStorage.getItem(COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}

/** A sidebar item: icon and label, or just the icon (with a tooltip). */
const NAV_ITEM =
  "flex min-h-9 w-full cursor-pointer items-center gap-3 rounded-lg px-2 text-[0.9375rem] font-medium text-muted-foreground transition-colors duration-100 hover:bg-[color-mix(in_srgb,var(--text)_6%,transparent)] hover:text-foreground active:bg-surface-press focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring motion-reduce:transition-none";

/** When collapsed only the icon shows: the label moves to a tooltip and
 *  stays the accessible name. */
function Labelled({
  collapsed,
  label,
  children,
}: {
  collapsed: boolean;
  label: string;
  children: ReactElement;
}) {
  return collapsed ? (
    <Tooltip label={label} side="right">
      {children}
    </Tooltip>
  ) : (
    children
  );
}

function NavButton(props: {
  label: string;
  icon: ReactNode;
  active: boolean;
  collapsed: boolean;
  onClick: () => void;
}) {
  return (
    <Labelled collapsed={props.collapsed} label={props.label}>
      <button
        type="button"
        className={cn(
          NAV_ITEM,
          "aria-[current=page]:bg-sidebar-accent aria-[current=page]:font-semibold aria-[current=page]:text-foreground",
          props.collapsed && "justify-center",
        )}
        aria-current={props.active ? "page" : undefined}
        aria-label={props.collapsed ? props.label : undefined}
        onClick={props.onClick}
      >
        {props.icon}
        {!props.collapsed && <span>{props.label}</span>}
      </button>
    </Labelled>
  );
}

/**
 * While efsp's firewall rules are on, a reminder stays in the sidebar on
 * every page; it opens Servers, where servers are switched back on.
 */
function RulesOn({ collapsed, open }: { collapsed: boolean; open: () => void }) {
  const picker = usePicker();
  if (!picker?.rulesOn) return null;
  const n = picker.off.length;
  const what = n > 0 ? `${String(n)} ${n === 1 ? "server" : "servers"} off` : "Firewall rules on";
  return (
    <Labelled collapsed={collapsed} label={what}>
      <button
        type="button"
        className={cn(NAV_ITEM, "text-left", collapsed && "justify-center")}
        aria-label={what}
        onClick={open}
      >
        <span className="mx-1.5 size-2 shrink-0 rounded-full bg-action" aria-hidden="true" />
        {!collapsed && <span>{what}</span>}
      </button>
    </Labelled>
  );
}

/** App shell: a collapsible sidebar (Live, History, Servers, Settings) and the page. */
export function Shell({ npcap }: { npcap: NpcapStatus }) {
  const [tab, setTab] = useState<Tab>("live");
  const [collapsed, setCollapsed] = useState(readCollapsed);
  const { error } = useSettings();

  const toggle = () => {
    const next = !collapsed;
    setCollapsed(next);
    try {
      localStorage.setItem(COLLAPSED_KEY, next ? "1" : "0");
    } catch {
      // Not remembered; harmless.
    }
  };

  return (
    // One rounded panel: the sidebar (a tone between the panel and the
    // window frame) and the page beside it.
    <div
      className={cn(
        "grid h-full min-h-0 overflow-hidden rounded-[0.625rem] bg-background",
        collapsed ? "grid-cols-[3.5rem_1fr]" : "grid-cols-[13rem_1fr]",
      )}
    >
      <nav
        className="flex min-h-0 flex-col gap-4 border-r border-line bg-sidebar px-2 py-4"
        aria-label="Main"
      >
        <div className={cn("flex min-h-9 items-center", collapsed && "justify-center")}>
          <button
            type="button"
            className={cn(NAV_ITEM, "w-auto justify-center")}
            aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
            aria-expanded={!collapsed}
            onClick={toggle}
          >
            <CollapseIcon collapsed={collapsed} />
          </button>
        </div>
        <div className="grid gap-1">
          <NavButton
            label="Live"
            icon={<LiveIcon />}
            active={tab === "live"}
            collapsed={collapsed}
            onClick={() => {
              setTab("live");
            }}
          />
          <NavButton
            label="History"
            icon={<HistoryIcon />}
            active={tab === "history"}
            collapsed={collapsed}
            onClick={() => {
              setTab("history");
            }}
          />
          <NavButton
            label="Servers"
            icon={<ServersIcon />}
            active={tab === "servers"}
            collapsed={collapsed}
            onClick={() => {
              setTab("servers");
            }}
          />
        </div>
        <div className="mt-auto">
          <RulesOn
            collapsed={collapsed}
            open={() => {
              setTab("servers");
            }}
          />
          <NavButton
            label="Settings"
            icon={<SettingsIcon />}
            active={tab === "settings"}
            collapsed={collapsed}
            onClick={() => {
              setTab("settings");
            }}
          />
        </div>
      </nav>

      <main className="scroll-panel min-w-0 overflow-y-auto px-8 pt-6 pb-8 max-[640px]:p-4">
        {error !== null && (
          <InlineError className="mb-4">Settings weren't saved: {error}</InlineError>
        )}
        {/* Live stays mounted so the card and graph never miss a tick. */}
        <div hidden={tab !== "live"}>
          <MonitorScreen
            npcap={npcap}
            onOpenServers={() => {
              setTab("servers");
            }}
          />
        </div>
        {tab === "history" && <HistoryView />}
        {tab === "servers" && <ServersView />}
        {tab === "settings" && <SettingsView />}
      </main>
    </div>
  );
}
