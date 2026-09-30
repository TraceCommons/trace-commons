import { Button as ButtonPrimitive } from "@base-ui/react/button"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "cn"

const buttonVariants = cva(
  "tc-btn group/button shrink-0 outline-none select-none disabled:pointer-events-none [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-3.5",
  {
    variants: {
      variant: {
        default: "tc-btn--primary tc-btn--sm",
        outline: "tc-btn--glass",
        secondary: "tc-btn--glass",
        ghost: "tc-btn--glass",
        destructive: "tc-btn--glass tc-text-outside",
        link: "tc-link",
      },
      size: {
        default: "",
        xs: "",
        sm: "",
        lg: "",
        icon: "tc-btn--round",
        "icon-xs": "tc-btn--round tc-btn--small",
        "icon-sm": "tc-btn--round tc-btn--small",
        "icon-lg": "tc-btn--round",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

function Button({
  className,
  variant = "default",
  size = "default",
  ...props
}: ButtonPrimitive.Props & VariantProps<typeof buttonVariants>) {
  return (
    <ButtonPrimitive
      data-slot="button"
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  )
}

export { Button, buttonVariants }
