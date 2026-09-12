import { Icon, type IconName } from "./icons";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ui/tooltip";

type IconButtonProps = {
  label: string;
  icon: IconName;
  onClick: () => void;
  pressed?: boolean;
  disabled?: boolean;
};

// `aria-label` is the accessible name; the hint on hover and on keyboard focus is the Tooltip,
// which replaces the native `title` and its half-second, system-styled popup.
export function IconButton({ label, icon, onClick, pressed, disabled }: IconButtonProps) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={label}
          aria-pressed={pressed}
          disabled={disabled}
          onClick={onClick}
          className={`grid size-8 place-items-center rounded-lg transition-[background-color,color,scale] duration-150 active:scale-95 disabled:opacity-35 ${
            pressed ? "bg-sidebar text-ink" : "text-ink-muted enabled:hover:bg-sidebar enabled:hover:text-ink"
          }`}
        >
          <Icon name={icon} />
        </button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
