interface BrandLogoProps {
  className?: string;
  /** Mark as decorative when an adjacent text label already names the brand. */
  decorative?: boolean;
}

export function BrandLogo({ className = "", decorative = false }: BrandLogoProps) {
  return (
    <img
      src="/brand/skillhive-logo.png"
      alt={decorative ? "" : "SkillHive"}
      aria-hidden={decorative || undefined}
      className={`brand-logo ${className}`.trim()}
      draggable={false}
    />
  );
}
