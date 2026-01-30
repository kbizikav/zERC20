// SPDX-License-Identifier: GPL-3.0
pragma solidity >=0.7.0 <0.9.0;

/*
    Sonobe's Nova + CycleFold decider verifier.
    Joint effort by 0xPARC & PSE.

    More details at https://github.com/privacy-scaling-explorations/sonobe
    Usage and design documentation at https://privacy-scaling-explorations.github.io/sonobe-docs/

    Uses the https://github.com/iden3/snarkjs/blob/master/templates/verifier_groth16.sol.ejs
    Groth16 verifier implementation and a KZG10 Solidity template adapted from
    https://github.com/weijiekoh/libkzg.
    Additionally we implement the RootNovaDecider contract, which combines the
    Groth16 and KZG10 verifiers to verify the zkSNARK proofs coming from
    Nova+CycleFold folding.
*/


/* =============================== */
/* KZG10 verifier methods */
/**
 * @author  Privacy and Scaling Explorations team - pse.dev
 * @dev     Contains utility functions for ops in BN254; in G_1 mostly.
 * @notice  Forked from https://github.com/weijiekoh/libkzg.
 * Among others, a few of the changes we did on this fork were:
 * - Templating the pragma version
 * - Removing type wrappers and use uints instead
 * - Performing changes on arg types
 * - Update some of the `require` statements 
 * - Use the bn254 scalar field instead of checking for overflow on the babyjub prime
 * - In batch checking, we compute auxiliary polynomials and their commitments at the same time.
 */
contract KZG10Verifier {

    // prime of field F_p over which y^2 = x^3 + 3 is defined
    uint256 public constant BN254_PRIME_FIELD =
        21888242871839275222246405745257275088696311157297823662689037894645226208583;
    uint256 public constant BN254_SCALAR_FIELD =
        21888242871839275222246405745257275088548364400416034343698204186575808495617;

    /**
     * @notice  Performs scalar multiplication in G_1.
     * @param   p  G_1 point to multiply
     * @param   s  Scalar to multiply by
     * @return  r  G_1 point p multiplied by scalar s
     */
    function mulScalar(uint256[2] memory p, uint256 s) internal view returns (uint256[2] memory r) {
        uint256[3] memory input;
        input[0] = p[0];
        input[1] = p[1];
        input[2] = s;
        bool success;
        assembly {
            success := staticcall(sub(gas(), 2000), 7, input, 0x60, r, 0x40)
            switch success
            case 0 { invalid() }
        }
        require(success, "bn254: scalar mul failed");
    }

    /**
     * @notice  Negates a point in G_1.
     * @param   p  G_1 point to negate
     * @return  uint256[2]  G_1 point -p
     */
    function negate(uint256[2] memory p) internal pure returns (uint256[2] memory) {
        if (p[0] == 0 && p[1] == 0) {
            return p;
        }
        return [p[0], BN254_PRIME_FIELD - (p[1] % BN254_PRIME_FIELD)];
    }

    /**
     * @notice  Adds two points in G_1.
     * @param   p1  G_1 point 1
     * @param   p2  G_1 point 2
     * @return  r  G_1 point p1 + p2
     */
    function add(uint256[2] memory p1, uint256[2] memory p2) internal view returns (uint256[2] memory r) {
        bool success;
        uint256[4] memory input = [p1[0], p1[1], p2[0], p2[1]];
        assembly {
            success := staticcall(sub(gas(), 2000), 6, input, 0x80, r, 0x40)
            switch success
            case 0 { invalid() }
        }

        require(success, "bn254: point add failed");
    }

    /**
     * @notice  Computes the pairing check e(p1, p2) * e(p3, p4) == 1
     * @dev     Note that G_2 points a*i + b are encoded as two elements of F_p, (a, b)
     * @param   a_1  G_1 point 1
     * @param   a_2  G_2 point 1
     * @param   b_1  G_1 point 2
     * @param   b_2  G_2 point 2
     * @return  result  true if pairing check is successful
     */
    function pairing(uint256[2] memory a_1, uint256[2][2] memory a_2, uint256[2] memory b_1, uint256[2][2] memory b_2)
        internal
        view
        returns (bool result)
    {
        uint256[12] memory input = [
            a_1[0],
            a_1[1],
            a_2[0][1], // imaginary part first
            a_2[0][0],
            a_2[1][1], // imaginary part first
            a_2[1][0],
            b_1[0],
            b_1[1],
            b_2[0][1], // imaginary part first
            b_2[0][0],
            b_2[1][1], // imaginary part first
            b_2[1][0]
        ];

        uint256[1] memory out;
        bool success;

        assembly {
            success := staticcall(sub(gas(), 2000), 8, input, 0x180, out, 0x20)
            switch success
            case 0 { invalid() }
        }

        require(success, "bn254: pairing failed");

        return out[0] == 1;
    }

    uint256[2] G_1 = [
            1991601893241931372498392980011332581235093909350275733045073223965815259151,
            911245099793270094425812359924167997708055137853481157556644025314520757924
    ];
    uint256[2][2] G_2 = [
        [
            258149005489619054183049368119392913550866370633088264718904325891731350597,
            20804620682363497903914449641796316804117219193712416604166474661272278162288
        ],
        [
            7271145842202950424317456359932198886050341345417220401693636326724699286626,
            5368423383224231372918728617314597787005194257999180244044268560706109088585
        ]
    ];
    uint256[2][2] VK = [
        [
            16199979717815984967427226758891560525267960121939882851509225366912533377099,
            6365610381281800743265003333695079629312830776942296746313965514996489210934
        ],
        [
            19818765041669429230688781518446693155858371144450432699841595886006925826491,
            17559086108396211396440471720138623948634140184689272806545414122191666306578
        ]
    ];

    

    /**
     * @notice  Verifies a single point evaluation proof. Function name follows `ark-poly`.
     * @dev     To avoid ops in G_2, we slightly tweak how the verification is done.
     * @param   c  G_1 point commitment to polynomial.
     * @param   pi G_1 point proof.
     * @param   x  Value to prove evaluation of polynomial at.
     * @param   y  Evaluation poly(x).
     * @return  result Indicates if KZG proof is correct.
     */
    function check(uint256[2] calldata c, uint256[2] calldata pi, uint256 x, uint256 y)
        public
        view
        returns (bool result)
    {
        //
        // we want to:
        //      1. avoid gas intensive ops in G2
        //      2. format the pairing check in line with what the evm opcode expects.
        //
        // we can do this by tweaking the KZG check to be:
        //
        //          e(pi, vk - x * g2) = e(c - y * g1, g2) [initial check]
        //          e(pi, vk - x * g2) * e(c - y * g1, g2)^{-1} = 1
        //          e(pi, vk - x * g2) * e(-c + y * g1, g2) = 1 [bilinearity of pairing for all subsequent steps]
        //          e(pi, vk) * e(pi, -x * g2) * e(-c + y * g1, g2) = 1
        //          e(pi, vk) * e(-x * pi, g2) * e(-c + y * g1, g2) = 1
        //          e(pi, vk) * e(x * -pi - c + y * g1, g2) = 1 [done]
        //                        |_   rhs_pairing  _|
        //
        uint256[2] memory rhs_pairing =
            add(mulScalar(negate(pi), x), add(negate(c), mulScalar(G_1, y)));
        return pairing(pi, VK, rhs_pairing, G_2);
    }

    function evalPolyAt(uint256[] memory _coefficients, uint256 _index) public pure returns (uint256) {
        uint256 m = BN254_SCALAR_FIELD;
        uint256 result = 0;
        uint256 powerOfX = 1;

        for (uint256 i = 0; i < _coefficients.length; i++) {
            uint256 coeff = _coefficients[i];
            assembly {
                result := addmod(result, mulmod(powerOfX, coeff, m), m)
                powerOfX := mulmod(powerOfX, _index, m)
            }
        }
        return result;
    }

    
}

/* =============================== */
/* Groth16 verifier methods */
/*
    Copyright 2021 0KIMS association.

    * `solidity-verifiers` added comment
        This file is a template built out of [snarkJS](https://github.com/iden3/snarkjs) groth16 verifier.
        See the original ejs template [here](https://github.com/iden3/snarkjs/blob/master/templates/verifier_groth16.sol.ejs)
    *

    snarkJS is a free software: you can redistribute it and/or modify it
    under the terms of the GNU General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    snarkJS is distributed in the hope that it will be useful, but WITHOUT
    ANY WARRANTY; without even the implied warranty of MERCHANTABILITY
    or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public
    License for more details.

    You should have received a copy of the GNU General Public License
    along with snarkJS. If not, see <https://www.gnu.org/licenses/>.
*/

contract Groth16Verifier {
    // Scalar field size
    uint256 constant r    = 21888242871839275222246405745257275088548364400416034343698204186575808495617;
    // Base field size
    uint256 constant q   = 21888242871839275222246405745257275088696311157297823662689037894645226208583;

    // Verification Key data
    uint256 constant alphax  = 13530871878073108532367910481049741800761901023548096971337412225870712944996;
    uint256 constant alphay  = 11127132771389376315802199914300146015440158946862548260200241915042058863415;
    uint256 constant betax1  = 17849726925422114740407772918480649013038128489585905392794390582218832975749;
    uint256 constant betax2  = 10930755745128634695615514946659140853327476894255013279674439321080798677133;
    uint256 constant betay1  = 3652265959209139255792130500907206167660666121227689467701736468979040314513;
    uint256 constant betay2  = 688640120189861334757141830004819323246791223187972529115190110594702548745;
    uint256 constant gammax1 = 783141628224704612714552046653490271467873055448873943019325939886119304032;
    uint256 constant gammax2 = 843404876473428746781379991695599236420798981201746560524779765638265612957;
    uint256 constant gammay1 = 20397157259835277863271023645830713876667666662461224388235975432578370360003;
    uint256 constant gammay2 = 11883928400616893650341998731259201366607840420650876300667083087605014547309;
    uint256 constant deltax1 = 9309525375328327316855744103926066033390161108650842661858806605308771507350;
    uint256 constant deltax2 = 3598329602060018966865430513295317990816629629532834575872614622501708412737;
    uint256 constant deltay1 = 7140306723869369157179243161192651871334814189493621391341253861503191063582;
    uint256 constant deltay2 = 9310053512095758852267752929071238446712377443291010269810584165580900770647;

    
    uint256 constant IC0x = 16015320738199645159739388474420293596075524683056408267283803728068369982315;
    uint256 constant IC0y = 19029389773430642923435962453195773147534198786625415906312777674803259303199;
    
    uint256 constant IC1x = 8689863812998076642769747867346028042233137876739302911843717587339520161234;
    uint256 constant IC1y = 12441300442174770441452174348802609859838593702033935115643444268279376223550;
    
    uint256 constant IC2x = 1709141994286214936853762210332147289890796145637726103058317811653426715901;
    uint256 constant IC2y = 5945430795782725139207485611375929068836984666259694220775382300847497400036;
    
    uint256 constant IC3x = 19396609905556784281614265427903756124969922573340863778974120121866658616201;
    uint256 constant IC3y = 4316235374765108998047055293481455106586349588356761274021979650811089693582;
    
    uint256 constant IC4x = 15011512920265876829510713847738181103239849276296131455632118503133158463196;
    uint256 constant IC4y = 2268621180395050703531317118503517874776867369324762672231629983108659954854;
    
    uint256 constant IC5x = 5540633916921454367187431781817493299295017276357562481705949791139399080410;
    uint256 constant IC5y = 10382453227903630716685339030000958419268502933706215531309165274751964943032;
    
    uint256 constant IC6x = 8841880905847921517239828499533697277169564552565140595225379085480116984730;
    uint256 constant IC6y = 12936719699568763959726223797788850454313156069676188038108740173351026071827;
    
    uint256 constant IC7x = 11174359848995947921300291195135016567966243730088815154269388567290481492720;
    uint256 constant IC7y = 9934769679451737552903826006130646552331069098037837503592624618841376117431;
    
    uint256 constant IC8x = 2565973181282636472086033639700934912567054282903215739234650287141461974365;
    uint256 constant IC8y = 13290248658743097612123714002228125414928909403922250821490793817091974256935;
    
    uint256 constant IC9x = 13710596060034528216258521112111656240433906111122132907329321261051349668960;
    uint256 constant IC9y = 21376920730213956691090832930470042548624746752719037310155512016159394691265;
    
    uint256 constant IC10x = 4568373601729686836601826512781426742585284429300457765728887945651919538443;
    uint256 constant IC10y = 10290632272054153434457547604871544117293920543194249804940020495252173315851;
    
    uint256 constant IC11x = 20789234244034374864535459082949955327389178837218631888749113126557446274516;
    uint256 constant IC11y = 7606955497780719084725923649445136534951392872157231698845223792043188103072;
    
    uint256 constant IC12x = 15494539981412824617485230519105309095654715519466938567483656455928889786840;
    uint256 constant IC12y = 10135274800109003214263840503205563332768693460377729033263943561157201345224;
    
    uint256 constant IC13x = 4970036178833475351291789723469172576634599866798432260196941539447493412519;
    uint256 constant IC13y = 16767140215825871393968277997509021041629467826103150488020270044067027981836;
    
    uint256 constant IC14x = 19007027447255310796170661477075397087271580427262855606763990796605157548737;
    uint256 constant IC14y = 4998804949831423278038285849346209062053168173279751470744303851106524817363;
    
    uint256 constant IC15x = 15280135599684741386666605994200435603745614397730112060701266096649020858607;
    uint256 constant IC15y = 8143433652680263712836186888851969058638811031970186326220072472712255862852;
    
    uint256 constant IC16x = 14157126378047396657667451097637695883923785465678215351028783109375751912661;
    uint256 constant IC16y = 1460499477453649246411858310437409381445860862129798719031951843362235609923;
    
    uint256 constant IC17x = 18006031014254419977491062936428299291191029275413079146171993410904280238803;
    uint256 constant IC17y = 6459621542568835777924889356546806888974136783130724274273137026988918635953;
    
    uint256 constant IC18x = 2950232718578313549242609634754448912721996710095351135570450152322235523917;
    uint256 constant IC18y = 19145083927985930331025057004872525062099197462042935762825490112668828394397;
    
    uint256 constant IC19x = 1771676585182509713152973518220116628011548811862099529911960054544343565709;
    uint256 constant IC19y = 1868165397137650570560221539810167204376718729871780103486680998306118934803;
    
    uint256 constant IC20x = 984962673569895828767496715019816342602018795566563205389629307482020223404;
    uint256 constant IC20y = 15621055898150403198871793741893505674011951168653070385890973034023055126317;
    
    uint256 constant IC21x = 897135952072891542281401237154472118185817208094724913254160581055392499009;
    uint256 constant IC21y = 1058228019674993048860336081989064607665824426644419171887188053542694321874;
    
    uint256 constant IC22x = 5788632813248864591664819440533631879712770054900063635252079602462749862013;
    uint256 constant IC22y = 20597054901255289177177184984668782774601623122070197671700650249814466496432;
    
    uint256 constant IC23x = 5987370884891297614167436674451464999228763308180587368584157315312362291633;
    uint256 constant IC23y = 14909392137384093039133989725296753425467743923132580753210688795327467577397;
    
    uint256 constant IC24x = 20509755473634933421492500088718391245311990890402117324070392202589582874626;
    uint256 constant IC24y = 14187544228583289463336562949298713857458334236982623367983421648830588277809;
    
    uint256 constant IC25x = 17718731724094745079282688476235540385200901766637203040509002867771388310207;
    uint256 constant IC25y = 8772196887261517542295295751183531425280163107041450901593636011035505011004;
    
    uint256 constant IC26x = 569827845623717124719063895004703077725875125074962870222026646859710812297;
    uint256 constant IC26y = 11254364969505730605552324630958559371735733029097596800996716154243278215309;
    
    uint256 constant IC27x = 8675516109660496104193091901302447089155798532153888692912129827990182417969;
    uint256 constant IC27y = 11701535816993855045263392759420191858160012898803249944683678404278416856333;
    
    uint256 constant IC28x = 7680073767119804376212296364478395870341073578140491664579652394476447929233;
    uint256 constant IC28y = 17356908107108429900689369751854953333469002871522066026082443085156670823039;
    
    uint256 constant IC29x = 6160922119369589171921594793794632009906620715329402466718228953350324008650;
    uint256 constant IC29y = 7444779376336241447170809049849647835830547476747545900038662077085144698396;
    
    uint256 constant IC30x = 13871574449870435307120685468856822539638856254239590687117584094570601087366;
    uint256 constant IC30y = 10444625726871295341151547810299244007240391047058292262150933370351588048235;
    
    uint256 constant IC31x = 16654544970537245712692679276398069940169754255623763811931074943886308852796;
    uint256 constant IC31y = 20216129291590085142597063104452522629961568299545240964127923614606692393206;
    
    uint256 constant IC32x = 10899009716459153496113837355692728635367635009569908780721033118748580443382;
    uint256 constant IC32y = 4863728994613991660462048949469242008071298018634100029729309994824739210390;
    
    uint256 constant IC33x = 9515076791297631558333501018107966792228605009836068350043228404783870552666;
    uint256 constant IC33y = 12602321404556759141588853695538488456595616854437284857603207043155022874279;
    
    uint256 constant IC34x = 15282880692396469668650383948839086508370628447802173893903057976110067271499;
    uint256 constant IC34y = 20121103794558355453386053257664685927521313172591251992612139990569387770597;
    
    uint256 constant IC35x = 18802769191732237733214025938753294906793307937384106478805735381647466354586;
    uint256 constant IC35y = 1838839358160538110398980473467099240966754531856271456363895879113823343329;
    
    uint256 constant IC36x = 845394370781125527192764149246570191681322681583647323513266354141693199295;
    uint256 constant IC36y = 11526406651630967530004675151913303685113955580431053416901941254224223384002;
    
    uint256 constant IC37x = 2359664652485129320710272321855468307935086487207352929910644799241098429636;
    uint256 constant IC37y = 15144013666598304163783233348799063596861954505770659049440122212482385442818;
    
    uint256 constant IC38x = 2815495807640454719575210082695863128443267160908648789443036626073515505257;
    uint256 constant IC38y = 14270139402118771457727268644751643960194965000100725697273689615547778593289;
    
    uint256 constant IC39x = 2647599548051325022138258314292681559114690690852299319603264373690361869872;
    uint256 constant IC39y = 12254534835380883073972136497977756871728245711745614428839746490611614695477;
    
    uint256 constant IC40x = 13723983427788606019686814907170796229321771874862156059126791930945124931683;
    uint256 constant IC40y = 7177502804191001056430212419078261311251952687361451293648162624043822241472;
    
    uint256 constant IC41x = 21143052624713323035397072351051469299140937895913446967910312660407911643479;
    uint256 constant IC41y = 6401177620777490255743136797915603618879115957511129362288935889044274159175;
    
    uint256 constant IC42x = 11460418243093913072204019227677820371827125222752437502698048849436633035997;
    uint256 constant IC42y = 16781165947299139717814584633934332847812860630947886842503093863339588429408;
    
    
    // Memory data
    uint16 constant pVk = 0;
    uint16 constant pPairing = 128;

    uint16 constant pLastMem = 896;

    function verifyProof(uint[2] calldata _pA, uint[2][2] calldata _pB, uint[2] calldata _pC, uint[42] calldata _pubSignals) public view returns (bool) {
        assembly {
            function checkField(v) {
                if iszero(lt(v, r)) {
                    mstore(0, 0)
                    return(0, 0x20)
                }
            }
            
            // G1 function to multiply a G1 value(x,y) to value in an address
            function g1_mulAccC(pR, x, y, s) {
                let success
                let mIn := mload(0x40)
                mstore(mIn, x)
                mstore(add(mIn, 32), y)
                mstore(add(mIn, 64), s)

                success := staticcall(sub(gas(), 2000), 7, mIn, 96, mIn, 64)

                if iszero(success) {
                    mstore(0, 0)
                    return(0, 0x20)
                }

                mstore(add(mIn, 64), mload(pR))
                mstore(add(mIn, 96), mload(add(pR, 32)))

                success := staticcall(sub(gas(), 2000), 6, mIn, 128, pR, 64)

                if iszero(success) {
                    mstore(0, 0)
                    return(0, 0x20)
                }
            }

            function checkPairing(pA, pB, pC, pubSignals, pMem) -> isOk {
                let _pPairing := add(pMem, pPairing)
                let _pVk := add(pMem, pVk)

                mstore(_pVk, IC0x)
                mstore(add(_pVk, 32), IC0y)

                // Compute the linear combination vk_x
                
                
                g1_mulAccC(_pVk, IC1x, IC1y, calldataload(add(pubSignals, 0)))
                g1_mulAccC(_pVk, IC2x, IC2y, calldataload(add(pubSignals, 32)))
                g1_mulAccC(_pVk, IC3x, IC3y, calldataload(add(pubSignals, 64)))
                g1_mulAccC(_pVk, IC4x, IC4y, calldataload(add(pubSignals, 96)))
                g1_mulAccC(_pVk, IC5x, IC5y, calldataload(add(pubSignals, 128)))
                g1_mulAccC(_pVk, IC6x, IC6y, calldataload(add(pubSignals, 160)))
                g1_mulAccC(_pVk, IC7x, IC7y, calldataload(add(pubSignals, 192)))
                g1_mulAccC(_pVk, IC8x, IC8y, calldataload(add(pubSignals, 224)))
                g1_mulAccC(_pVk, IC9x, IC9y, calldataload(add(pubSignals, 256)))
                g1_mulAccC(_pVk, IC10x, IC10y, calldataload(add(pubSignals, 288)))
                g1_mulAccC(_pVk, IC11x, IC11y, calldataload(add(pubSignals, 320)))
                g1_mulAccC(_pVk, IC12x, IC12y, calldataload(add(pubSignals, 352)))
                g1_mulAccC(_pVk, IC13x, IC13y, calldataload(add(pubSignals, 384)))
                g1_mulAccC(_pVk, IC14x, IC14y, calldataload(add(pubSignals, 416)))
                g1_mulAccC(_pVk, IC15x, IC15y, calldataload(add(pubSignals, 448)))
                g1_mulAccC(_pVk, IC16x, IC16y, calldataload(add(pubSignals, 480)))
                g1_mulAccC(_pVk, IC17x, IC17y, calldataload(add(pubSignals, 512)))
                g1_mulAccC(_pVk, IC18x, IC18y, calldataload(add(pubSignals, 544)))
                g1_mulAccC(_pVk, IC19x, IC19y, calldataload(add(pubSignals, 576)))
                g1_mulAccC(_pVk, IC20x, IC20y, calldataload(add(pubSignals, 608)))
                g1_mulAccC(_pVk, IC21x, IC21y, calldataload(add(pubSignals, 640)))
                g1_mulAccC(_pVk, IC22x, IC22y, calldataload(add(pubSignals, 672)))
                g1_mulAccC(_pVk, IC23x, IC23y, calldataload(add(pubSignals, 704)))
                g1_mulAccC(_pVk, IC24x, IC24y, calldataload(add(pubSignals, 736)))
                g1_mulAccC(_pVk, IC25x, IC25y, calldataload(add(pubSignals, 768)))
                g1_mulAccC(_pVk, IC26x, IC26y, calldataload(add(pubSignals, 800)))
                g1_mulAccC(_pVk, IC27x, IC27y, calldataload(add(pubSignals, 832)))
                g1_mulAccC(_pVk, IC28x, IC28y, calldataload(add(pubSignals, 864)))
                g1_mulAccC(_pVk, IC29x, IC29y, calldataload(add(pubSignals, 896)))
                g1_mulAccC(_pVk, IC30x, IC30y, calldataload(add(pubSignals, 928)))
                g1_mulAccC(_pVk, IC31x, IC31y, calldataload(add(pubSignals, 960)))
                g1_mulAccC(_pVk, IC32x, IC32y, calldataload(add(pubSignals, 992)))
                g1_mulAccC(_pVk, IC33x, IC33y, calldataload(add(pubSignals, 1024)))
                g1_mulAccC(_pVk, IC34x, IC34y, calldataload(add(pubSignals, 1056)))
                g1_mulAccC(_pVk, IC35x, IC35y, calldataload(add(pubSignals, 1088)))
                g1_mulAccC(_pVk, IC36x, IC36y, calldataload(add(pubSignals, 1120)))
                g1_mulAccC(_pVk, IC37x, IC37y, calldataload(add(pubSignals, 1152)))
                g1_mulAccC(_pVk, IC38x, IC38y, calldataload(add(pubSignals, 1184)))
                g1_mulAccC(_pVk, IC39x, IC39y, calldataload(add(pubSignals, 1216)))
                g1_mulAccC(_pVk, IC40x, IC40y, calldataload(add(pubSignals, 1248)))
                g1_mulAccC(_pVk, IC41x, IC41y, calldataload(add(pubSignals, 1280)))
                g1_mulAccC(_pVk, IC42x, IC42y, calldataload(add(pubSignals, 1312)))

                // -A
                mstore(_pPairing, calldataload(pA))
                mstore(add(_pPairing, 32), mod(sub(q, calldataload(add(pA, 32))), q))

                // B
                mstore(add(_pPairing, 64), calldataload(pB))
                mstore(add(_pPairing, 96), calldataload(add(pB, 32)))
                mstore(add(_pPairing, 128), calldataload(add(pB, 64)))
                mstore(add(_pPairing, 160), calldataload(add(pB, 96)))

                // alpha1
                mstore(add(_pPairing, 192), alphax)
                mstore(add(_pPairing, 224), alphay)

                // beta2
                mstore(add(_pPairing, 256), betax1)
                mstore(add(_pPairing, 288), betax2)
                mstore(add(_pPairing, 320), betay1)
                mstore(add(_pPairing, 352), betay2)

                // vk_x
                mstore(add(_pPairing, 384), mload(add(pMem, pVk)))
                mstore(add(_pPairing, 416), mload(add(pMem, add(pVk, 32))))


                // gamma2
                mstore(add(_pPairing, 448), gammax1)
                mstore(add(_pPairing, 480), gammax2)
                mstore(add(_pPairing, 512), gammay1)
                mstore(add(_pPairing, 544), gammay2)

                // C
                mstore(add(_pPairing, 576), calldataload(pC))
                mstore(add(_pPairing, 608), calldataload(add(pC, 32)))

                // delta2
                mstore(add(_pPairing, 640), deltax1)
                mstore(add(_pPairing, 672), deltax2)
                mstore(add(_pPairing, 704), deltay1)
                mstore(add(_pPairing, 736), deltay2)


                let success := staticcall(sub(gas(), 2000), 8, _pPairing, 768, _pPairing, 0x20)

                isOk := and(success, mload(_pPairing))
            }

            let pMem := mload(0x40)
            mstore(0x40, add(pMem, pLastMem))

            // Validate that all evaluations ∈ F
            
            checkField(calldataload(add(_pubSignals, 0)))
            
            checkField(calldataload(add(_pubSignals, 32)))
            
            checkField(calldataload(add(_pubSignals, 64)))
            
            checkField(calldataload(add(_pubSignals, 96)))
            
            checkField(calldataload(add(_pubSignals, 128)))
            
            checkField(calldataload(add(_pubSignals, 160)))
            
            checkField(calldataload(add(_pubSignals, 192)))
            
            checkField(calldataload(add(_pubSignals, 224)))
            
            checkField(calldataload(add(_pubSignals, 256)))
            
            checkField(calldataload(add(_pubSignals, 288)))
            
            checkField(calldataload(add(_pubSignals, 320)))
            
            checkField(calldataload(add(_pubSignals, 352)))
            
            checkField(calldataload(add(_pubSignals, 384)))
            
            checkField(calldataload(add(_pubSignals, 416)))
            
            checkField(calldataload(add(_pubSignals, 448)))
            
            checkField(calldataload(add(_pubSignals, 480)))
            
            checkField(calldataload(add(_pubSignals, 512)))
            
            checkField(calldataload(add(_pubSignals, 544)))
            
            checkField(calldataload(add(_pubSignals, 576)))
            
            checkField(calldataload(add(_pubSignals, 608)))
            
            checkField(calldataload(add(_pubSignals, 640)))
            
            checkField(calldataload(add(_pubSignals, 672)))
            
            checkField(calldataload(add(_pubSignals, 704)))
            
            checkField(calldataload(add(_pubSignals, 736)))
            
            checkField(calldataload(add(_pubSignals, 768)))
            
            checkField(calldataload(add(_pubSignals, 800)))
            
            checkField(calldataload(add(_pubSignals, 832)))
            
            checkField(calldataload(add(_pubSignals, 864)))
            
            checkField(calldataload(add(_pubSignals, 896)))
            
            checkField(calldataload(add(_pubSignals, 928)))
            
            checkField(calldataload(add(_pubSignals, 960)))
            
            checkField(calldataload(add(_pubSignals, 992)))
            
            checkField(calldataload(add(_pubSignals, 1024)))
            
            checkField(calldataload(add(_pubSignals, 1056)))
            
            checkField(calldataload(add(_pubSignals, 1088)))
            
            checkField(calldataload(add(_pubSignals, 1120)))
            
            checkField(calldataload(add(_pubSignals, 1152)))
            
            checkField(calldataload(add(_pubSignals, 1184)))
            
            checkField(calldataload(add(_pubSignals, 1216)))
            
            checkField(calldataload(add(_pubSignals, 1248)))
            
            checkField(calldataload(add(_pubSignals, 1280)))
            
            checkField(calldataload(add(_pubSignals, 1312)))
            
            checkField(calldataload(add(_pubSignals, 1344)))
            

            // Validate all evaluations
            let isValid := checkPairing(_pA, _pB, _pC, _pubSignals, pMem)

            mstore(0, isValid)
            
            return(0, 0x20)
        }
    }
}


/* =============================== */
/* Nova+CycleFold Decider verifier */
/**
 * @notice  Computes the decomposition of a `uint256` into num_limbs limbs of bits_per_limb bits each.
 * @dev     Compatible with sonobe::folding-schemes::folding::circuits::nonnative::nonnative_field_to_field_elements.
 */
library LimbsDecomposition {
    function decompose(uint256 x) internal pure returns (uint256[5] memory) {
        uint256[5] memory limbs;
        for (uint8 i = 0; i < 5; i++) {
            limbs[i] = (x >> (55 * i)) & ((1 << 55) - 1);
        }
        return limbs;
    }
}

/**
 * @author PSE & 0xPARC
 * @title  Interface for the RootNovaDecider contract hiding proof details.
 * @dev    This interface enables calling the verifyNovaProof function without exposing the proof details.
 */
interface OpaqueDecider {
    /**
     * @notice  Verifies a Nova+CycleFold proof given initial and final IVC states, number of steps and the rest proof inputs concatenated.
     * @dev     This function should simply reorganize arguments and pass them to the proper verification function.
     */
    function verifyOpaqueNovaProofWithInputs(
        uint256 steps, // number of folded steps (i)
        uint256[3] calldata initial_state, // initial IVC state (z0)
        uint256[3] calldata final_state, // IVC state after i steps (zi)
        uint256[25] calldata proof // the rest of the decider inputs
    ) external view returns (bool);

    /**
     * @notice  Verifies a Nova+CycleFold proof given all the proof inputs collected in a single array.
     * @dev     This function should simply reorganize arguments and pass them to the proper verification function.
     */
    function verifyOpaqueNovaProof(uint256[32] calldata proof) external view returns (bool);
}

/**
 * @author  PSE & 0xPARC
 * @title   RootNovaDecider contract, for verifying Nova IVC SNARK proofs.
 * @dev     This is an askama template which, when templated, features a Groth16 and KZG10 verifiers from which this contract inherits.
 */
contract RootNovaDecider is Groth16Verifier, KZG10Verifier, OpaqueDecider {
    /**
     * @notice  Computes the linear combination of a and b with r as the coefficient.
     * @dev     All ops are done mod the BN254 scalar field prime
     */
    function rlc(uint256 a, uint256 r, uint256 b) internal pure returns (uint256 result) {
        assembly {
            result := addmod(a, mulmod(r, b, BN254_SCALAR_FIELD), BN254_SCALAR_FIELD)
        }
    }

    /**
     * @notice  Verifies a nova cyclefold proof consisting of two KZG proofs and of a groth16 proof.
     * @dev     The selector of this function is "dynamic", since it depends on `z_len`.
     */
    function verifyNovaProof(
        // inputs are grouped to prevent errors due stack too deep
        uint256[7] calldata i_z0_zi, // [i, z0, zi] where |z0| == |zi|
        uint256[4] calldata U_i_cmW_U_i_cmE, // [U_i_cmW[2], U_i_cmE[2]]
        uint256[2] calldata u_i_cmW, // [u_i_cmW[2]]
        uint256[3] calldata cmT_r, // [cmT[2], r]
        uint256[2] calldata pA, // groth16 
        uint256[2][2] calldata pB, // groth16
        uint256[2] calldata pC, // groth16
        uint256[4] calldata challenge_W_challenge_E_kzg_evals, // [challenge_W, challenge_E, eval_W, eval_E]
        uint256[2][2] calldata kzg_proof // [proof_W, proof_E]
    ) public view returns (bool) {

        require(i_z0_zi[0] >= 2, "Folding: the number of folded steps should be at least 2");

        // from gamma_abc_len, we subtract 1. 
        uint256[42] memory public_inputs; 

        public_inputs[0] = 173885680870925996246966120200108614841136518852629338239561859619805295181;
        public_inputs[1] = i_z0_zi[0];

        for (uint i = 0; i < 6; i++) {
            public_inputs[2 + i] = i_z0_zi[1 + i];
        }

        {
            // U_i.cmW + r * u_i.cmW
            uint256[2] memory mulScalarPoint = super.mulScalar([u_i_cmW[0], u_i_cmW[1]], cmT_r[2]);
            uint256[2] memory cmW = super.add([U_i_cmW_U_i_cmE[0], U_i_cmW_U_i_cmE[1]], mulScalarPoint);

            {
                uint256[5] memory cmW_x_limbs = LimbsDecomposition.decompose(cmW[0]);
                uint256[5] memory cmW_y_limbs = LimbsDecomposition.decompose(cmW[1]);
        
                for (uint8 k = 0; k < 5; k++) {
                    public_inputs[8 + k] = cmW_x_limbs[k];
                    public_inputs[13 + k] = cmW_y_limbs[k];
                }
            }
        
            require(this.check(cmW, kzg_proof[0], challenge_W_challenge_E_kzg_evals[0], challenge_W_challenge_E_kzg_evals[2]), "KZG: verifying proof for challenge W failed");
        }

        {
            // U_i.cmE + r * cmT
            uint256[2] memory mulScalarPoint = super.mulScalar([cmT_r[0], cmT_r[1]], cmT_r[2]);
            uint256[2] memory cmE = super.add([U_i_cmW_U_i_cmE[2], U_i_cmW_U_i_cmE[3]], mulScalarPoint);

            {
                uint256[5] memory cmE_x_limbs = LimbsDecomposition.decompose(cmE[0]);
                uint256[5] memory cmE_y_limbs = LimbsDecomposition.decompose(cmE[1]);
            
                for (uint8 k = 0; k < 5; k++) {
                    public_inputs[18 + k] = cmE_x_limbs[k];
                    public_inputs[23 + k] = cmE_y_limbs[k];
                }
            }

            require(this.check(cmE, kzg_proof[1], challenge_W_challenge_E_kzg_evals[1], challenge_W_challenge_E_kzg_evals[3]), "KZG: verifying proof for challenge E failed");
        }

        {
            // add challenges
            public_inputs[28] = challenge_W_challenge_E_kzg_evals[0];
            public_inputs[29] = challenge_W_challenge_E_kzg_evals[1];
            public_inputs[30] = challenge_W_challenge_E_kzg_evals[2];
            public_inputs[31] = challenge_W_challenge_E_kzg_evals[3];

            uint256[5] memory cmT_x_limbs;
            uint256[5] memory cmT_y_limbs;
        
            cmT_x_limbs = LimbsDecomposition.decompose(cmT_r[0]);
            cmT_y_limbs = LimbsDecomposition.decompose(cmT_r[1]);
        
            for (uint8 k = 0; k < 5; k++) {
                public_inputs[28 + 4 + k] = cmT_x_limbs[k]; 
                public_inputs[33 + 4 + k] = cmT_y_limbs[k];
            }

            bool success_g16 = this.verifyProof(pA, pB, pC, public_inputs);
            require(success_g16 == true, "Groth16: verifying proof failed");
        }

        return(true);
    }

    /**
     * @notice  Verifies a Nova+CycleFold proof given initial and final IVC states, number of steps and the rest proof inputs concatenated.
     * @dev     Simply reorganization of arguments and call to the `verifyNovaProof` function.
     */
    function verifyOpaqueNovaProofWithInputs(
        uint256 steps,
        uint256[3] calldata initial_state,
        uint256[3] calldata final_state,
        uint256[25] calldata proof
    ) public override view returns (bool) {
        uint256[1 + 2 * 3] memory i_z0_zi;
        i_z0_zi[0] = steps;
        for (uint256 i = 0; i < 3; i++) {
            i_z0_zi[i + 1] = initial_state[i];
            i_z0_zi[i + 1 + 3] = final_state[i];
        }

        uint256[4] memory U_i_cmW_U_i_cmE = [proof[0], proof[1], proof[2], proof[3]];
        uint256[2] memory u_i_cmW = [proof[4], proof[5]];
        uint256[3] memory cmT_r = [proof[6], proof[7], proof[8]];
        uint256[2] memory pA = [proof[9], proof[10]];
        uint256[2][2] memory pB = [[proof[11], proof[12]], [proof[13], proof[14]]];
        uint256[2] memory pC = [proof[15], proof[16]];
        uint256[4] memory challenge_W_challenge_E_kzg_evals = [proof[17], proof[18], proof[19], proof[20]];
        uint256[2][2] memory kzg_proof = [[proof[21], proof[22]], [proof[23], proof[24]]];

        return this.verifyNovaProof(
            i_z0_zi,
            U_i_cmW_U_i_cmE,
            u_i_cmW,
            cmT_r,
            pA,
            pB,
            pC,
            challenge_W_challenge_E_kzg_evals,
            kzg_proof
        );
    }

    /**
     * @notice  Verifies a Nova+CycleFold proof given all proof inputs concatenated.
     * @dev     Simply reorganization of arguments and call to the `verifyNovaProof` function.
     */
    function verifyOpaqueNovaProof(uint256[32] calldata proof) public override view returns (bool) {
        uint256[3] memory z0;
        uint256[3] memory zi;
        for (uint256 i = 0; i < 3; i++) {
            z0[i] = proof[i + 1];
            zi[i] = proof[i + 1 + 3];
        }

        uint256[25] memory extracted_proof;
        for (uint256 i = 0; i < 25; i++) {
            extracted_proof[i] = proof[7 + i];
        }

        return this.verifyOpaqueNovaProofWithInputs(proof[0], z0, zi, extracted_proof);
    }
}