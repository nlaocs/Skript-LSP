# Explicit maintenance tool: review the complete output diff; never overwrite raw observations.
from pathlib import Path
import json

out = Path(__file__).parent
baseline = json.loads((out / 'expected.json').read_text(encoding='utf-8'))
cases = json.loads((out / 'cases.json').read_text(encoding='utf-8'))['cases']
graphs = json.loads((out / 'upstream-graphs.json').read_text(encoding='utf-8'))
requests = []
for graph in graphs:
    matches = []
    for case in cases:
        if case['fixture'] != 'skript-2.16.2-java25':
            continue
        expression = case['mode'] == 'expression'
        text = 'send ' + case['input'] + ' to console' if expression else case['input']
        event = 'on load' if expression else case['event']
        if graph['source'] == str(event) + ':\n\t' + text:
            matches.append({'id': case['id'], 'input': text, 'event': event, 'expression_wrapper': expression})
    if len(matches) != 1:
        raise RuntimeError('Each observed source must map to exactly one curated case')
    requests.extend(matches)
assert len(requests) == len(graphs) == 7

def pointer(value, path):
    for segment in path.strip('/').split('/'):
        value = value[int(segment)] if isinstance(value, list) else value[segment]
    return value

observations = []
for request, graph in zip(requests, graphs):
    assert graph['valid'] is True and graph['validationException'] is None
    assert graph['source'] == request['event'] + ':\n\t' + request['input']
    assert len(graph['returnedLists']) == 1 and len(graph['returnedLists'][0]) == 1
    nodes = {node['id']: node for node in graph['nodes']}
    root = nodes[graph['returnedLists'][0][0]]
    native = baseline[request['id']]
    checks, differences = [], []
    unknown = list(graph['unknown'])

    def check(path, expected, mode='value'):
        actual = pointer(native, path)
        if mode == 'length': actual = len(actual)
        if mode == 'sourceNumber':
            expected = float(expected)
            # Fixed small integer sources only, not runtime evaluation.
            case_input = next(case['input'] for case in json.loads((out / 'cases.json').read_text(encoding='utf-8'))['cases'] if case['id'] == request['id'])
            actual = float(case_input[actual[0]:actual[1]])
        item = {'path': path, 'mode': mode, 'expected': expected}
        if actual == expected:
            checks.append(item)
        else:
            if actual in ('ch.njol.skript.classes.ClassInfo', ['ch.njol.skript.classes.ClassInfo']) and expected in ('org.skriptlang.skript.bukkit.types.PlayerClassInfo', ['org.skriptlang.skript.bukkit.types.PlayerClassInfo']):
                reason = 'Native ClassInfo abstraction versus observed runtime PlayerClassInfo implementation type; normalization not specified and not a compatibility pass'
            else:
                raise RuntimeError(f'Unexpected upstream discrepancy must be reviewed: {request["id"]} {path}: {actual!r} != {expected!r}')
            item.update({'current_native': actual, 'reason': reason})
            differences.append(item)

    def flatten(node_id):
        node = nodes[node_id]
        values = [edge['child'] for edge in node['edges'] if edge['role'].startswith('listValue:')]
        return sum((flatten(value) for value in values), []) if values else [node_id]

    def expression(node_id, path):
        observed = nodes[node_id]
        n = pointer(native, path)
        if 'ConvertedExpression' in observed['class']:
            source = next((edge['child'] for edge in observed['edges'] if edge['role'] == 'conversionSource'), None)
            unknown.append(path + ': physical conversion consumed type/path not represented natively; source expression compared separately')
            if source is not None:
                expression(source, path)
            return
        if 'getReturnType' in observed: check(path + '/returnType', observed['getReturnType'])
        if 'isSingle' in observed: check(path + '/multiplicity', 'Single' if observed['isSingle'] else 'Multiple')
        if 'possibleReturnTypes' in observed: check(path + '/possibleTypes', observed['possibleReturnTypes'])
        if 'function' in observed:
            check(path + '/expression/kind', 'function')
            check(path + '/function/name', observed['function'])
            arguments = observed['suppliedArguments']
            check(path + '/function/arguments', len(arguments), 'length')
            start = 0
            bindings = {binding['name']: binding['value'] for binding in observed.get('parameterBindings', [])}
            for index, argument in enumerate(arguments):
                p = path + f'/function/arguments/{index}'
                default = nodes[argument['value']].get('isDefault', False)
                values = [] if default else flatten(bindings.get(argument['name'], argument['value']))
                check(p + '/parameter', argument['name'])
                check(p + '/omitted', default)
                check(p + '/childStart', start)
                check(p + '/childCount', len(values))
                check(p + '/suppliedName', argument['name'] if argument['kind'] == 'NAMED' else None)
                for offset, value in enumerate(values): expression(value, path + f'/children/{start + offset}')
                start += len(values)
        elif observed.get('isDefault'):
            check(path + '/expression/kind', 'default')
        elif 'acceptedInitPattern' in observed:
            if n['expression']['kind'] == 'registered':
                check(path + '/expression/class', observed['class'])
                check(path + '/expression/identity/patternIndex', observed['acceptedInitPattern'])
                slots = observed['acceptedInitCaptures']
                if len(slots) == len(n['captures']):
                    for index, child in enumerate(slots):
                        if child is not None and n['captures'][index]['resolved'] is not None:
                            expression(child, path + f'/captures/{index}/resolved')
                        elif child is None:
                            check(path + f'/captures/{index}/resolved', None)
                else: unknown.append(path + ': accepted-init/native capture mapping unavailable')
            else: unknown.append(path + ': registered identity normalization unavailable')
        elif 'literalValues' in observed and len(observed['literalValues']) == 1 and isinstance(observed['literalValues'][0], (int, float)):
            check(path + '/expression/kind', 'literal')
            check(path + '/span', observed['literalValues'][0], 'sourceNumber')
        else: unknown.append(path + ': deeper child/identity normalization unknown for ' + observed['class'])

    if request['expression_wrapper']:
        expression(root['acceptedInitCaptures'][0], '/root')
    else:
        check('/root/selected/identity/class', root['class'])
        check('/root/selected/identity/patternIndex', root['acceptedInitPattern'])
        check('/root/captures', len(root['acceptedInitCaptures']), 'length')
        for index, child in enumerate(root['acceptedInitCaptures']):
            path = f'/root/captures/{index}'
            check(path + '/index', index)
            if child is None:
                check(path + '/state', 'Null')
                check(path + '/resolved', None)
            else:
                check(path + '/state', 'Default' if nodes[child].get('isDefault') else 'Explicit')
                expression(child, path + '/resolved')
    observations.append({'case_id': request['id'], 'graph_index': len(observations), 'expression_wrapper': request['expression_wrapper'], 'coverage': 'partial observed fields only', 'checks': checks, 'known_differences': differences, 'unknown': sorted(set(unknown))})

evidence = {'stage': 'actual outer loadItems returned objects inspected after validateScript return', 'snapshot_id': 'f42091ee442583229f11a2deea6b56ec736acb0c094ac4f4f01ab36b72192ba8', 'skript_jar_sha256': '8ed7254a69f6fc2a9b665b9d45233480719c31ac1483b5834ee14936e2a10ef3', 'ssg_jar_sha256': '8fc84d9b93a33e0d14ed1273e21f8459891119d816e1c6e6369f0fba4b549bb2', 'observations': observations}
(out / 'upstream.json').write_text(json.dumps(evidence, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
print(json.dumps({'checks': sum(len(o['checks']) for o in observations), 'differences': [{'case': o['case_id'], 'items': o['known_differences']} for o in observations if o['known_differences']], 'unknown_counts': {o['case_id']: len(o['unknown']) for o in observations}}, ensure_ascii=True))
