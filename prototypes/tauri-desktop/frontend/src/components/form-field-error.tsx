export function FormFieldError({
  id,
  message,
}: {
  id: string;
  message?: string;
}) {
  if (!message) return null;
  return (
    <p id={id} role="alert" className="m-0 text-xs text-destructive">
      {message}
    </p>
  );
}
