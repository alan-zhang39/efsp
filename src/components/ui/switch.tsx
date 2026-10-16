import { Switch as SwitchPrimitive } from "@base-ui/react/switch";
import { cn } from "cn";

/** An on/off switch: green when on, the thumb settling with a short ease. */
function Switch({ className, ...props }: SwitchPrimitive.Root.Props) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      // A real <button>: native disabled state, and a wrapping <label> works.
      nativeButton
      render={<button type="button" />}
      className={cn(
        "relative inline-flex h-[1.375rem] w-9 shrink-0 cursor-pointer items-center rounded-full bg-line transition-colors duration-150 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring data-checked:bg-ready data-disabled:cursor-default data-disabled:opacity-45 motion-reduce:transition-none",
        className,
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        data-slot="switch-thumb"
        className="pointer-events-none block size-4 translate-x-[0.1875rem] rounded-full bg-background transition-transform duration-[180ms] ease-[cubic-bezier(0.22,1,0.36,1)] data-checked:translate-x-[1.0625rem] motion-reduce:transition-none"
      />
    </SwitchPrimitive.Root>
  );
}

export { Switch };
