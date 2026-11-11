// Short global names for the Zest types the pages use. Zest publishes the
// declarations; this file only gives the names.
import type { CheckBox, ComboBox, ControlOptionInput, FormControl, PushButton, SpinBox, TextArea, TextField } from "@catalystcommunity/controls";
import type { DialogWindow, TabView } from "@catalystcommunity/containers";
import type { MenuButton } from "@catalystcommunity/menus";

declare global {
  type ZestControl = FormControl;
  type ZestPushButton = PushButton;
  type ZestTextField = TextField;
  type ZestTextArea = TextArea;
  type ZestCheckBox = CheckBox;
  type ZestComboBox = ComboBox;
  type ZestSpinBox = SpinBox;
  type ZestTabView = TabView;
  type ZestDialog = DialogWindow;
  type ZestMenuButton = MenuButton;
  /** One choice of a combo box. A string is both the value and the label. */
  type ZestOption = ControlOptionInput;
}
