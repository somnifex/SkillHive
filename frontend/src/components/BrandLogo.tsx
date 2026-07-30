interface BrandLogoProps {
  className?: string;
}

export function BrandLogo({ className = "" }: BrandLogoProps) {
  return (
    <img
      src="/brand/skillhive-logo.png"
      alt="SkillHive"
      className={`brand-logo ${className}`.trim()}
      draggable={false}
    />
  );
}
