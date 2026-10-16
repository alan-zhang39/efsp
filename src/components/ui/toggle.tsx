import { Toggle as TogglePrimitive } from "@base-ui/react/toggle";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "cn";

const toggleVariants = cva(
  "inline-flex cursor-pointer items-center justify-center whitespace-nowrap rounded-md text-sm font-medium text-muted-foreground select-none focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring data-pressed:bg-surface data-pressed:text-foreground disabled:cursor-default disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        /** One option inside a segmented control. */
        segment: "px-3 py-1.5 data-pressed:font-semibold",
        /** An icon option inside a segmented control. */
        icon: "px-[0.45rem] py-[0.3rem]",
        /** A standalone on/off chip, e.g. "Offers". */
        chip: "gap-1.5 rounded-lg border border-line px-3 py-1.5 font-normal focus-visible:outline-offset-2",
      },
    },
    defaultVariants: {
      variant: "chip",
    },
  },
);

function Toggle({
  className,
  variant,
  ...props
}: TogglePrimitive.Props & VariantProps<typeof toggleVariants>) {
  return (
    <TogglePrimitive
      data-slot="toggle"
      className={cn(toggleVariants({ variant }), className)}
      {...props}
    />
  );
}

export { Toggle, toggleVariants };
