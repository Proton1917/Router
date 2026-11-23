def normalize(settings, payload, percent):
    config = payload.get('config')
    if not isinstance(config, dict):
        raise ValueError('上游未返回此账号的订阅配额')
    period = config.get('currentPeriod', {})
    used = config.get('creditUsagePercent')
    # protobuf JSON 在已有计费周期时省略值为零的字段。
    if used is None and period.get('type'):
        used = 0
    remaining = None if used is None else max(0, 100 - float(used))
    title = settings.get('period_labels', {}).get(period.get('type'), '当前周期')
    result = {'groups': [{'label': '订阅共享额度' if config.get('isUnifiedBillingUser') else 'Grok Build 额度',
                         'windows': [{'label': title, 'remaining_percent': percent(remaining),
                                      'reset_at': period.get('end') or config.get('billingPeriodEnd')}]}], 'details': []}
    for item in config.get('productUsage', []):
        result['details'].append({'label': settings.get('product_labels', {}).get(item['product'], item['product']) + ' 已用',
                                  'value': str(percent(item.get('usagePercent', 0))) + '%'})
    for field, label in settings.get('balance_labels', {}).items():
        if field in config:
            result['details'].append({'label': label, 'value': f"${int(config[field].get('val', 0)) / 100:.2f} USD"})
    return result
