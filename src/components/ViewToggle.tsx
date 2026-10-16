import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Tooltip } from "@/components/ui/tooltip";
import type { HistoryView as View } from "../lib/settings";
import { CardsIcon, RowsIcon } from "./icons";

const VIEWS = [
  { value: "rows", label: "Rows", icon: <RowsIcon /> },
  { value: "cards", label: "Cards", icon: <CardsIcon /> },
] as const;

/** Rows or cards, as icons; the choice is remembered in settings. */
export function ViewToggle({
  value,
  onChange,
  label,
}: {
  value: View;
  onChange: (view: View) => void;
  /** e.g. "Show history as". */
  label: string;
}) {
  return (
    <ToggleGroup label={label} value={value} onValueChange={onChange}>
      {VIEWS.map((v) => (
        <Tooltip key={v.value} label={v.label}>
          <ToggleGroupItem icon value={v.value} aria-label={v.label}>
            {v.icon}
          </ToggleGroupItem>
        </Tooltip>
      ))}
    </ToggleGroup>
  );
}
