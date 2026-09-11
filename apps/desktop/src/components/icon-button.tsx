import { Icon, type IconName } from "./icons";

type IconButtonProps = {
  label: string;
  icon: IconName;
  onClick: () => void;
  pressed?: boolean;
  disabled?: boolean;
};

export function IconButton({ label, icon, onClick, pressed, disabled }: IconButtonProps) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      aria-pressed={pressed}
      disabled={disabled}
      onClick={onClick}
      className={`grid size-8 place-items-center rounded-lg transition-[background-color,color,scale] duration-150 active:scale-95 disabled:opacity-35 ${
        pressed ? "bg-sidebar text-ink" : "text-ink-muted enabled:hover:bg-sidebar enabled:hover:text-ink"
      }`}
    >
      <Icon name={icon} />
    </button>
  );
}
