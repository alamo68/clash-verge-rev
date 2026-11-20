import type { ChangeEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { Switch, TooltipIcon } from '@/components/base'
import { useVerge } from '@/hooks/use-verge'

import { GuardState } from './mods/guard-state'
import { SettingItem, SettingList } from './mods/setting-comp'

interface Props {
  onError: (err: Error) => void
}

const SettingHearthstone = ({ onError }: Props) => {
  const { t } = useTranslation()
  const { verge, mutateVerge, patchVerge } = useVerge()

  return (
    <SettingList title={t('settings.sections.hearthstone.title')}>
      <SettingItem
        data-setting-target="settings.sections.hearthstone.disconnect"
        label={t('settings.sections.hearthstone.disconnect')}
        extra={
          <TooltipIcon
            title={t('settings.sections.hearthstone.description')}
            sx={{ opacity: '0.7' }}
          />
        }
      >
        <GuardState
          value={verge?.enable_hearthstone_disconnect ?? true}
          valueProps="checked"
          onCatch={onError}
          onFormat={(_event: ChangeEvent<HTMLInputElement>, checked: boolean) =>
            checked
          }
          onChange={(enabled) =>
            mutateVerge((current) => ({
              ...current,
              enable_hearthstone_disconnect: enabled,
            }))
          }
          onGuard={(enabled) =>
            patchVerge({ enable_hearthstone_disconnect: enabled })
          }
        >
          <Switch
            edge="end"
            slotProps={{
              input: {
                'aria-label': t('settings.sections.hearthstone.disconnect'),
              },
            }}
          />
        </GuardState>
      </SettingItem>
    </SettingList>
  )
}

export default SettingHearthstone
