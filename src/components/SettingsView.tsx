import { cn } from "cn";
import { useId, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { deleteAllHistory } from "../lib/history";
import { KEEP_DAYS_OPTIONS, type Settings, type Show, type Theme } from "../lib/settings";
import { useSettings } from "../lib/SettingsContext";
import { Hint, Page, PageHeader } from "./Page";

const THEMES: { value: Theme; label: string }[] = [
  { value: "dark", label: "Dark" },
  { value: "light", label: "Light" },
  { value: "system", label: "Match Windows" },
];

function keepLabel(days: number | null): string {
  return days === null ? "Forever" : `${String(days)} days`;
}

/** A labelled group of rows. */
function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="grid gap-1" aria-label={title}>
      <h2 className="m-0 text-[0.8125rem] font-medium text-muted-foreground">{title}</h2>
      <div className="grid">{children}</div>
    </section>
  );
}

/** One row: the label on the left, its control on the right. */
const ROW =
  "flex min-h-11 items-center justify-between gap-4 border-t border-line text-[0.9375rem] first:border-t-0";

/** A row with a switch on the right; the whole row toggles it. */
function SwitchRow(props: {
  label: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (on: boolean) => void;
}) {
  const id = useId();
  return (
    <label className={cn(ROW, props.disabled === true && "text-muted-foreground")}>
      <span id={id}>{props.label}</span>
      <Switch
        aria-labelledby={id}
        checked={props.checked}
        disabled={props.disabled === true}
        onCheckedChange={props.onChange}
      />
    </label>
  );
}

/** A row with a choice on the right. */
function SelectRow<T extends string>(props: {
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
}) {
  return (
    <div className={ROW}>
      <span>{props.label}</span>
      <Select
        align="end"
        label={props.label}
        value={props.value}
        options={props.options}
        onValueChange={props.onChange}
      />
    </div>
  );
}

/** Settings: one grouped list; every change saves at once. */
export function SettingsView() {
  const { settings, update } = useSettings();
  const [deleteResult, setDeleteResult] = useState<string | null>(null);

  const set = (patch: Partial<Settings>) => {
    void update({ ...settings, ...patch });
  };
  const setShow = (patch: Partial<Show>) => {
    void update({ ...settings, show: { ...settings.show, ...patch } });
  };

  const deleteAll = () => {
    setDeleteResult(null);
    deleteAllHistory().then(
      (n) => {
        if (n !== null) setDeleteResult(`Deleted ${String(n)} matches.`);
      },
      (e: unknown) => {
        setDeleteResult(e instanceof Error ? e.message : String(e));
      },
    );
  };

  const keepOptions = KEEP_DAYS_OPTIONS.map((d) => ({
    value: d === null ? "forever" : String(d),
    label: keepLabel(d),
  }));

  return (
    <Page className="max-w-xl">
      <PageHeader title="Settings" />

      <Group title="Appearance">
        <SelectRow
          label="Theme"
          value={settings.theme}
          options={THEMES}
          onChange={(theme) => {
            set({ theme });
          }}
        />
      </Group>

      <Group title="Startup">
        <SwitchRow
          label="Start with Windows"
          checked={settings.startWithWindows}
          onChange={(on) => {
            set({ startWithWindows: on, startMinimised: on && settings.startMinimised });
          }}
        />
        <SwitchRow
          label="Start in the tray"
          checked={settings.startMinimised}
          disabled={!settings.startWithWindows}
          onChange={(on) => {
            set({ startMinimised: on });
          }}
        />
      </Group>

      <Group title="Match card">
        <SwitchRow
          label="Jitter"
          checked={settings.show.jitter}
          onChange={(on) => {
            setShow({ jitter: on });
          }}
        />
        <SwitchRow
          label="Loss"
          checked={settings.show.loss}
          onChange={(on) => {
            setShow({ loss: on });
          }}
        />
        <SwitchRow
          label="Packets per second"
          checked={settings.show.packets}
          onChange={(on) => {
            setShow({ packets: on });
          }}
        />
      </Group>

      <Group title="Graph">
        <SwitchRow
          label="Jitter range"
          checked={settings.show.graphJitter}
          onChange={(on) => {
            setShow({ graphJitter: on });
          }}
        />
        <SwitchRow
          label="Lost pings"
          checked={settings.show.graphLosses}
          onChange={(on) => {
            setShow({ graphLosses: on });
          }}
        />
        <SwitchRow
          label="Packets per second"
          checked={settings.show.graphPackets}
          onChange={(on) => {
            setShow({ graphPackets: on });
          }}
        />
      </Group>

      <Group title="History">
        <SelectRow
          label="Keep matches for"
          value={settings.keepHistoryDays === null ? "forever" : String(settings.keepHistoryDays)}
          options={keepOptions}
          onChange={(v) => {
            set({ keepHistoryDays: v === "forever" ? null : Number(v) });
          }}
        />
        <div className={ROW}>
          <Button variant="danger" onClick={deleteAll}>
            Delete all history…
          </Button>
        </div>
      </Group>
      {deleteResult !== null && <Hint>{deleteResult}</Hint>}

      <p className="m-0 text-xs text-muted-foreground">
        IP geolocation by DB-IP (db-ip.com), CC BY 4.0.
      </p>
    </Page>
  );
}
