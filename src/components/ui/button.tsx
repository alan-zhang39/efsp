import { Button as ButtonPrimitive } from "@base-ui/react/button";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "cn";

/**
 * efsp's buttons. Feedback is on press (a small scale and a darker fill),
 * not on release; reduced motion keeps the colour and drops the scale.
 */
const buttonVariants = cva(
  "inline-flex shrink-0 cursor-pointer items-center justify-center gap-1.5 whitespace-nowrap select-none rounded-lg border font-semibold transition-[transform,background-color,opacity] duration-100 ease-out focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring active:not-disabled:scale-[0.97] disabled:cursor-default disabled:text-muted-foreground motion-reduce:transition-none motion-reduce:active:not-disabled:scale-100 [&_svg]:pointer-events-none [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default: "border-line bg-surface text-foreground active:not-disabled:bg-surface-press",
        primary:
          "border-transparent bg-primary text-primary-foreground active:not-disabled:opacity-90",
        quiet:
          "border-transparent bg-transparent text-muted-foreground hover:text-foreground active:not-disabled:bg-surface-press",
        /** Looks like text: back links and quiet inline actions. */
        text: "border-0 bg-transparent p-0 font-normal text-muted-foreground hover:text-foreground active:not-disabled:scale-100 focus-visible:rounded-sm",
        /** Text in the warning colour, for actions that delete. */
        danger:
          "border-0 bg-transparent p-0 font-normal text-poor active:not-disabled:scale-100 focus-visible:rounded-sm",
      },
      size: {
        default: "px-4 py-[0.5625rem] text-[0.9375rem]",
        text: "text-[0.875rem]",
        /** The surrounding text's size. */
        inline: "",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

function Button({
  className,
  variant = "default",
  size,
  ...props
}: ButtonPrimitive.Props & VariantProps<typeof buttonVariants>) {
  const fallback = variant === "text" ? "text" : variant === "danger" ? "inline" : "default";
  return (
    <ButtonPrimitive
      data-slot="button"
      className={cn(buttonVariants({ variant, size: size ?? fallback }), className)}
      {...props}
    />
  );
}

export { Button, buttonVariants };
