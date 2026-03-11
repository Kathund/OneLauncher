import { Dropdown } from '@onelauncher/common/components';

export interface SettingDropdownProps<T> {
	setting: [T, (value: T) => void];
	options: Array<{ key: T; label?: string }>;
}

export function SettingDropdown<T extends string>({ setting, options }: SettingDropdownProps<T>) {
	return (
		<Dropdown onSelectionChange={key => setting[1](key as T)} selectedKey={setting[0]}>
			{options.map(option => <Dropdown.Item id={option.key} key={option.key}>{option.label ?? option.key}</Dropdown.Item>)}
		</Dropdown>
	);
}
