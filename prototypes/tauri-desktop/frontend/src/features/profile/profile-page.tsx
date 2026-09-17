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
    const published = await actions.publish(values.handle, values.bio);
    if (published) {
      form.reset(values);
      setSaved(true);
    }
  };

  return (
    <FormProvider {...form}>
      <div className="mx-auto max-w-[1080px] px-4 pb-12 pt-8 sm:px-8 sm:pb-16 sm:pt-10 lg:px-16 lg:pt-14 block">
        <header className="mb-[38px] flex items-start justify-between gap-6">
          <div>
            <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
              ACCOUNT / PROFILE
            </span>
            <h1>Your profile</h1>
            <p>A small public surface for work you choose to share.</p>
          </div>
          <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
            PHASE 1
          </span>
        </header>
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
