export function SettingRow({
  label,
  value,
  detail,
}: {
  label: string;
  value: string;
  detail: string;
}) {
  return (
    <div className="flex items-center justify-between gap-6 border-t border-border py-[15px] first:mt-5">
      <div>
        <strong>{label}</strong>
        <span>{detail}</span>
      </div>
      <b>{value}</b>
    </div>
  );
}
