import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect, useState } from "react";
import { useForm } from "react-hook-form";
import { type ProfileFormValues, profileFormSchema } from "../forms";
import type { ProfileDraft, PublicProfile } from "../types";

const emptyDraft: ProfileDraft = { handle: "", bio: "" };

export function useProfileDraft(publicProfile: PublicProfile | null) {
  const form = useForm<ProfileFormValues>({
    resolver: zodResolver(profileFormSchema),
    defaultValues: emptyDraft,
  });
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    if (!publicProfile?.on_roster || form.formState.isDirty) return;
    form.reset({
      handle: publicProfile.handle ?? "",
      bio: publicProfile.bio ?? "",
    });
  }, [form, publicProfile]);
  const draft = form.watch();

  function update(field: keyof ProfileDraft, value: string) {
    setSaved(false);
    form.setValue(field, value, { shouldDirty: true });
  }

  function save() {
    setSaved(true);
    form.reset(form.getValues());
  }

  return { draft, saved, update, save, form };
}
