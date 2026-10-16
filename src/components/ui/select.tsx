import { Select as SelectPrimitive } from "@base-ui/react/select";
import { cn } from "cn";
import { CheckIcon, ChevronDownIcon } from "@/components/icons";

/**
 * A dropdown that looks like a chip. `options` keeps call sites short; the
 * trigger shows the chosen option's label. `label` names it for screen
 * readers (the chip itself shows only the value).
 */
function Select<T extends string>({
  value,
  onValueChange,
  options,
  label,
  align = "start",
  className,
}: {
  value: T;
  onValueChange: (value: T) => void;
  options: readonly { value: T; label: string }[];
  label: string;
  /** Which edge of the chip the list lines up with. */
  align?: "start" | "end";
  className?: string | undefined;
}) {
  return (
    <SelectPrimitive.Root
      value={value}
      items={options}
      onValueChange={(v) => {
        const picked = options.find((o) => o.value === v);
        if (picked !== undefined) onValueChange(picked.value);
      }}
    >
      <SelectPrimitive.Trigger
        data-slot="select-trigger"
        aria-label={label}
        className={cn(
          "inline-flex w-fit cursor-pointer items-center justify-between gap-1.5 rounded-lg border border-line bg-background py-1.5 pr-2 pl-2.5 text-sm leading-normal whitespace-nowrap text-foreground select-none focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring data-popup-open:bg-surface",
          className,
        )}
      >
        <SelectPrimitive.Value />
        <SelectPrimitive.Icon
          render={
            <ChevronDownIcon className="pointer-events-none size-3.5 text-muted-foreground" />
          }
        />
      </SelectPrimitive.Trigger>
      <SelectPrimitive.Portal>
        <SelectPrimitive.Positioner
          side="bottom"
          align={align}
          sideOffset={4}
          alignItemWithTrigger={false}
          className="isolate z-50"
        >
          <SelectPrimitive.Popup
            data-slot="select-content"
            className="max-h-(--available-height) min-w-(--anchor-width) origin-(--transform-origin) overflow-y-auto rounded-lg border border-line bg-popover p-1 text-popover-foreground shadow-lg shadow-black/25 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95 motion-reduce:animate-none"
          >
            <SelectPrimitive.List>
              {options.map((o) => (
                <SelectPrimitive.Item
                  key={o.value}
                  value={o.value}
                  data-slot="select-item"
                  className="relative flex cursor-pointer items-center rounded-md py-1.5 pr-8 pl-2 text-sm whitespace-nowrap outline-none select-none data-highlighted:bg-surface-press data-selected:font-semibold"
                >
                  <SelectPrimitive.ItemText>{o.label}</SelectPrimitive.ItemText>
                  <SelectPrimitive.ItemIndicator className="absolute right-2 flex size-4 items-center justify-center">
                    <CheckIcon className="size-3.5" />
                  </SelectPrimitive.ItemIndicator>
                </SelectPrimitive.Item>
              ))}
            </SelectPrimitive.List>
          </SelectPrimitive.Popup>
        </SelectPrimitive.Positioner>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  );
}

export { Select };
