export type ConsentOption = {
  name: string;
  /** The core's label for the scope; never derived from `name`. */
  title: string;
  description: string;
  always_on: boolean;
  grants_data_use: boolean;
};
