export type ConsentOption = {
  name: string;
  /** The core's label for the scope; never derived from `name`. */
  title: string;
  description: string;
  always_on: boolean;
  grants_data_use: boolean;
  /**
   * The core's short tag beside the title ("required" and the like), or
   * null from a daemon that sends none; never one this shell chose.
   */
  tag: string | null;
};
