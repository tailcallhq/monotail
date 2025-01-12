```graphql @schema
schema @server(port: 8000, hostname: "0.0.0.0") {
  query: Query
}

type Query {
  queryNodeC: [NodeC!] @graphQL(url: "http://upstream/graphql", name: "nodeC")
  queryNodeA: [NodeA!] @graphQL(url: "http://upstream/graphql", name: "nodeA")
  queryNodeB: [NodeB!] @graphQL(url: "http://upstream/graphql", name: "nodeB")
}

type NodeA implements NodeC {
  name: String
  nodeA_id: String
}

type NodeB implements NodeC {
  name: String
  nodeB_id: String
}

interface NodeC {
  name: String
}
```

```yml @mock
- request:
    method: POST
    url: http://upstream/graphql
    textBody: {"query": "query { nodeC { name __typename ...on NodeA { nodeA_id } ...on NodeB { nodeB_id } } }"}
  expectedHits: 1
  response:
    status: 200
    body:
      data:
        nodeC:
          - name: nodeA
            __typename: NodeA
            nodeA_id: nodeA_id
          - name: nodeB
            __typename: NodeB
            nodeB_id: nodeB_id
```

```yml @test
- method: POST
  url: http://localhost:8080/graphql
  body:
    query: |
      query queryNodeC {
        queryNodeC {
          name
          __typename
          ...on NodeA {
            nodeA_id
          }
          ...on NodeB {
            nodeB_id
          }
        }
      }
```
