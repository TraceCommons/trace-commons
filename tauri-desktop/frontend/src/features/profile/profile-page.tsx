import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect, useState } from "react";
import { FormProvider, useForm } from "react-hook-form";
import { ProfileEditor } from "./components/profile-editor";
import { ProfileSummary } from "./components/profile-summary";
import { PublicProfileConsent } from "./components/public-profile-consent";
import { type ProfileFormValues, profileFormSchema } from "./forms";
import { useProfileActions } from "./hooks/use-profile-actions";
import type { ProfilePageProps } from "./types";

export function ProfilePage({
  coreStatus,
  coreStatusState,
  onRefresh,
  publicProfile,
  publicProfileState,
  onPublicProfileRefresh,
}: ProfilePageProps) {
  const actions = useProfileActions();
  const [consentOpen, setConsentOpen] = useState(false);
  const [saved, setSaved] = useState(false);
  const form = useForm<ProfileFormValues>({
    resolver: zodResolver(profileFormSchema),
    defaultValues: { handle: "", bio: "" },
    mode: "onChange",
  });
  useEffect(() => {
    if (publicProfile?.on_roster && !form.formState.isDirty) {
      form.reset({
        handle: publicProfile.handle ?? "",
        bio: publicProfile.bio ?? "",
      });
    }
  }, [
    form,
    publicProfile?.bio,
    publicProfile?.handle,
    publicProfile?.on_roster,
  ]);
  const publishCurrent = async () => {
    const values = form.getValues();
    const result = await actions.publish(values.handle, values.bio);
    if (result) {
      form.reset(values);
      setSaved(true);
    }
  };

  return (
    <FormProvider {...form}>
      <div className="tc-page">
        <p className="m-0 tc-label font-normal leading-[17px] tc-text-secondary">
          A small public surface for work you choose to share.
        </p>
        <ProfileSummary
          coreStatus={coreStatus}
          coreStatusState={coreStatusState}
          onRefresh={async () => {
            await Promise.all([onRefresh(), onPublicProfileRefresh()]);
          }}
          profile={publicProfile}
          profileState={publicProfileState}
        />
        <ProfileEditor
          saved={saved}
          actionState={actions.state}
          actionError={actions.error}
          actionNotice={actions.notice}
          published={publicProfile?.on_roster ?? false}
          onSave={() => setSaved(true)}
          onPublish={() => {
            if (publicProfile?.on_roster) void publishCurrent();
            else setConsentOpen(true);
          }}
          onWithdraw={() => void actions.withdraw()}
        />
        <PublicProfileConsent
          open={consentOpen}
          handle={form.watch("handle")}
          bio={form.watch("bio")}
          busy={actions.state === "publishing"}
          error={consentOpen ? actions.error : null}
          onConfirm={() => {
            setConsentOpen(false);
            void publishCurrent();
          }}
          onCancel={() => setConsentOpen(false)}
        />
      </div>
    </FormProvider>
  );
}
