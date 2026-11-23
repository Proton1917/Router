body_credentials = {'project': 'project_id'}


def normalize(settings, payload, percent):
    groups = []
    details = []
    for group in payload['groups']:
        windows = [{'label': settings.get('window_labels', {}).get(bucket.get('window'), bucket.get('displayName', '配额')),
                    'remaining_percent': percent(bucket.get('remainingFraction'), 100),
                    'reset_at': bucket.get('resetTime'), 'disabled': bucket.get('disabled', False)}
                   for bucket in group.get('buckets', [])]
        name = group['displayName']
        label = settings.get('group_labels', {}).get(name, name)
        groups.append({'label': label, 'windows': windows})
        prefix = 'Models within this group: '
        description = group.get('description') or ''
        if description.startswith(prefix):
            members = description[len(prefix):].replace(', ', '、')
            details.append({'label': f'{label}组内模型', 'value': members})
    if not groups:
        raise ValueError('上游未返回配额分组')
    return {'groups': groups, 'details': details}
