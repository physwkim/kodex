use std::collections::HashMap;
use std::path::Path;

use crate::graph::EngramGraph;
use crate::security::sanitize_label;
use super::{node_community_map, js_safe, COMMUNITY_COLORS};

const MAX_NODES_FOR_VIZ: usize = 5000;

/// Export graph to interactive HTML visualization using vis.js.
pub fn to_html(
    graph: &EngramGraph,
    communities: &HashMap<usize, Vec<String>>,
    output_path: &Path,
    community_labels: Option<&HashMap<usize, String>>,
) -> Result<(), crate::error::EngramError> {
    if graph.node_count() > MAX_NODES_FOR_VIZ {
        return Err(crate::error::EngramError::Other(format!(
            "Graph has {} nodes (max {MAX_NODES_FOR_VIZ} for visualization)",
            graph.node_count()
        )));
    }

    let node_comm = node_community_map(communities);
    let max_deg = graph
        .node_ids()
        .map(|id| graph.degree(id))
        .max()
        .unwrap_or(1)
        .max(1) as f64;

    // Build vis nodes
    let vis_nodes: Vec<serde_json::Value> = graph
        .node_ids()
        .filter_map(|id| {
            let node = graph.get_node(id)?;
            let cid = node_comm.get(id).copied().unwrap_or(0);
            let color = COMMUNITY_COLORS[cid % COMMUNITY_COLORS.len()];
            let label = sanitize_label(&node.label);
            let deg = graph.degree(id);
            let size = 10.0 + 30.0 * (deg as f64 / max_deg);
            let font_size = if deg as f64 >= max_deg * 0.15 { 12 } else { 0 };
            let comm_name = community_labels
                .and_then(|cl| cl.get(&cid))
                .cloned()
                .unwrap_or_else(|| format!("Community {cid}"));

            Some(serde_json::json!({
                "id": id,
                "label": label,
                "color": { "background": color, "border": color, "highlight": { "background": color, "border": "#fff" } },
                "size": (size * 10.0).round() / 10.0,
                "font": { "size": font_size, "color": "#ccc" },
                "community": cid,
                "community_name": comm_name,
                "source_file": node.source_file,
                "file_type": node.file_type.to_string(),
                "degree": deg,
            }))
        })
        .collect();

    // Build vis edges
    let vis_edges: Vec<serde_json::Value> = graph
        .edges()
        .map(|(src, tgt, edge)| {
            let conf = edge.confidence.to_string();
            let is_extracted = edge.confidence == crate::types::Confidence::EXTRACTED;
            serde_json::json!({
                "from": src,
                "to": tgt,
                "label": edge.relation,
                "title": format!("{} [{}]", edge.relation, conf),
                "dashes": !is_extracted,
                "width": if is_extracted { 2 } else { 1 },
                "color": { "opacity": if is_extracted { 0.7 } else { 0.35 } },
            })
        })
        .collect();

    // Build legend
    let mut legend: Vec<serde_json::Value> = Vec::new();
    let mut sorted_cids: Vec<usize> = communities.keys().copied().collect();
    sorted_cids.sort();
    for cid in sorted_cids {
        let color = COMMUNITY_COLORS[cid % COMMUNITY_COLORS.len()];
        let label = community_labels
            .and_then(|cl| cl.get(&cid))
            .cloned()
            .unwrap_or_else(|| format!("Community {cid}"));
        let count = communities.get(&cid).map(|v| v.len()).unwrap_or(0);
        legend.push(serde_json::json!({
            "cid": cid, "color": color, "label": label, "count": count,
        }));
    }

    let nodes_json = js_safe(&serde_json::to_string(&vis_nodes).unwrap_or_default());
    let edges_json = js_safe(&serde_json::to_string(&vis_edges).unwrap_or_default());
    let legend_json = js_safe(&serde_json::to_string(&legend).unwrap_or_default());
    let hyperedges_json = js_safe(
        &serde_json::to_string(&graph.hyperedges).unwrap_or_else(|_| "[]".to_string()),
    );

    let stats = format!(
        "{} nodes &middot; {} edges &middot; {} communities",
        graph.node_count(),
        graph.edge_count(),
        communities.len()
    );

    let html = format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>engram</title>
<script src="https://unpkg.com/vis-network/standalone/umd/vis-network.min.js"></script>
<style>
body {{ margin:0; background:#1a1a2e; color:#ccc; font-family:system-ui; display:flex; }}
#graph {{ flex:1; height:100vh; }}
#sidebar {{ width:320px; padding:16px; overflow-y:auto; background:#16213e; border-left:1px solid #333; }}
input {{ width:100%; padding:8px; background:#1a1a2e; border:1px solid #444; color:#ccc; border-radius:4px; margin-bottom:12px; box-sizing:border-box; }}
.legend-item {{ cursor:pointer; padding:4px 0; display:flex; align-items:center; gap:8px; }}
.legend-dot {{ width:12px; height:12px; border-radius:50%; display:inline-block; }}
h3 {{ margin:16px 0 8px; color:#eee; font-size:14px; }}
.stats {{ color:#888; font-size:12px; margin-bottom:12px; }}
#info {{ font-size:13px; line-height:1.5; }}
</style>
</head>
<body>
<div id="graph"></div>
<div id="sidebar">
  <h3>engram</h3>
  <div class="stats">{stats}</div>
  <input id="search" placeholder="Search nodes..." />
  <h3>Communities</h3>
  <div id="legend"></div>
  <h3>Details</h3>
  <div id="info">Click a node to inspect.</div>
</div>
<script>
var nodesData={nodes_json};
var edgesData={edges_json};
var legendData={legend_json};
var hyperedgesData={hyperedges_json};
var nodes=new vis.DataSet(nodesData);
var edges=new vis.DataSet(edgesData);
var container=document.getElementById('graph');
var data={{nodes:nodes,edges:edges}};
var options={{
  physics:{{solver:'forceAtlas2Based',forceAtlas2Based:{{gravitationalConstant:-50,centralGravity:0.005,springLength:100}},stabilization:{{iterations:200}}}},
  interaction:{{hover:true,tooltipDelay:200,hideEdgesOnDrag:true}},
  nodes:{{shape:'dot',borderWidth:1.5}},
  edges:{{smooth:{{type:'continuous',roundness:0.2}},selectionWidth:3}}
}};
var network=new vis.Network(container,data,options);
// Search
document.getElementById('search').addEventListener('input',function(e){{
  var q=e.target.value.toLowerCase();
  if(!q){{nodes.forEach(function(n){{nodes.update({{id:n.id,hidden:false}})}});;return;}}
  nodes.forEach(function(n){{nodes.update({{id:n.id,hidden:!n.label.toLowerCase().includes(q)}})}});
}});
// Legend
function esc(s){{var d=document.createElement('div');d.textContent=s;return d.innerHTML;}}
var leg=document.getElementById('legend');
legendData.forEach(function(c){{
  var d=document.createElement('div');d.className='legend-item';
  var dot=document.createElement('span');dot.className='legend-dot';dot.style.background=c.color;
  d.appendChild(dot);d.appendChild(document.createTextNode(c.label+' ('+c.count+')'));
  d.onclick=function(){{
    nodes.forEach(function(n){{nodes.update({{id:n.id,hidden:n.community!==c.cid}})}});
  }};
  leg.appendChild(d);
}});
// Click
network.on('click',function(p){{
  if(!p.nodes.length)return;
  var nid=p.nodes[0],n=nodes.get(nid);
  var neighbors=network.getConnectedNodes(nid);
  var info='<b>'+esc(n.label)+'</b><br>File: '+esc(n.source_file)+'<br>Type: '+esc(n.file_type)+'<br>Community: '+esc(n.community_name)+'<br>Degree: '+n.degree;
  info+='<br><br><b>Neighbors ('+neighbors.length+'):</b><ul>';
  neighbors.slice(0,20).forEach(function(nb){{var nn=nodes.get(nb);if(nn)info+='<li>'+esc(nn.label)+'</li>';}});
  if(neighbors.length>20)info+='<li>... +'+(neighbors.length-20)+' more</li>';
  info+='</ul>';
  document.getElementById('info').innerHTML=info;
}});
</script>
</body>
</html>"##
    );

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output_path, html).map_err(Into::into)
}
