export type ComputeSnapshot = {
  state: string;
  reason: string;
  title: string;
  detail: string;
  consent_granted: boolean;
  ram_allowance_gib: number | null;
  available: boolean;
  can_enable: boolean;
  can_resume: boolean;
  can_pause: boolean;
  worker_stopped: boolean;
  copy: {
    introduction: string;
    allowance_label: string;
    allowance_detail: string;
    enable: string;
    resume: string;
    pause: string;
    disable: string;
  };
};
